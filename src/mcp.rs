//! Local stdio MCP-style server (hand-rolled JSON-RPC 2.0, no SDK).
//!
//! Read-only by construction: the only filesystem reads are source-file
//! analysis input, an optional coverage file, and the `git` reads already
//! performed inside [`crate::diff::analyze_changes`]. No shell, no network,
//! and no writes except the opt-in local metrics store ([`crate::telemetry`]),
//! which stays inside `LEADLINE_METRICS_DIR` and never touches the analyzed
//! repository.
//!
//! Wire format: newline-delimited JSON-RPC 2.0 over stdin/stdout.
//! Requests look like `{jsonrpc:"2.0", id, method, params}`; responses are
//! `{jsonrpc:"2.0", id, result}` or `{jsonrpc:"2.0", id, error:{code,message}}`.
//! Messages without an `id` are notifications and produce no response.

use crate::agent::{SortKey, changed_causes};
use crate::config::RegressionLimits;
use crate::core::{
    FunctionAnalysis, FunctionMetrics, METRIC_PROFILE, OUTPUT_SCHEMA_VERSION, Thresholds,
};
use std::collections::{BTreeSet, HashMap};
use std::io::BufRead as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Maximum entries returned in any result array before truncation kicks in.
const MAX_ENTRIES: usize = 200;

/// Maximum requests accepted in one JSON-RPC batch.
const MAX_BATCH_REQUESTS: usize = 64;

/// Maximum serialized bytes of one JSON-RPC response, batch included.
const MAX_RESPONSE_BYTES: usize = 32 << 20;

/// Usage guidance returned by `initialize`. Hosts may inject this into the
/// system prompt, so it doubles as the server's self-advertisement.
/// Trigger-first: name the user intent, then the tool. Keep every tool named
/// so hosts can route to it.
const SERVER_INSTRUCTIONS: &str = "Always use leadline for Java, JavaScript, TypeScript, TSX questions about complexity, maintainability, risk, hotspots, or change impact: deterministic function-level metrics without executing code or touching the network. Call analyze_changed after editing code, fixing bugs, refactoring, or before committing to spot regressions (before/after deltas; metrics are evidence, not objectives — do not refactor solely to lower a number). Call repo_summary first on unfamiliar code, onboarding, or triage; call risk plus impact plus coupling before editing a file to gauge change risk, blast radius, and hidden co-change contracts. Call analyze_function with explain:true for line-level causes (explain_metric before interpreting numbers), check to gate thresholds or regressions (pass coverage so CRAP gates are meaningful). Call hotspots for churn hotspots, test_targets (needs coverage) to pick what to test, dependencies for the static graph, duplication for token clones, policy for leadline.toml rules, debt to compare full state against a base, and project for the canonical summary when targeted tools are not enough. Call sql_plan to compare checked-in PostgreSQL EXPLAIN artifacts, security_findings to triage scanner SARIF with code context (needs pre-generated SARIF), vulnerabilities to prioritize deps with changed-import evidence (needs pre-generated OSV/Trivy), and sql_risks to flag static PostgreSQL risks — all read-only, never run scanners, databases, or network; secrets use native leadline_secret_check. The analyze and check tools accept an optional index directory and only ever read it. Batch independent calls in one block; start from repo_summary or project before per-function drills.";

/// The twenty tools this server exposes. Fixed set; keep in sync with
/// [`tools_list`] and [`dispatch_tool`].
const TOOL_NAMES: [&str; 20] = [
    "analyze",
    "analyze_changed",
    "analyze_function",
    "check",
    "explain_metric",
    "repo_summary",
    "security_findings",
    "sql_plan",
    "test_targets",
    "vulnerabilities",
    "sql_risks",
    "dependencies",
    "impact",
    "coupling",
    "hotspots",
    "duplication",
    "policy",
    "risk",
    "debt",
    "project",
];

/// Serve JSON-RPC requests from stdin, writing responses to stdout.
///
/// One request per line. Batch arrays are accepted when a whole line parses
/// as a JSON array. Blank lines are ignored. Responses are flushed after
/// every line: live clients keep stdin open while waiting, so buffering
/// until EOF would deadlock them into a request timeout. Lines past
/// [`MAX_STDIO_LINE`] fail instead of growing a buffer without bound.
pub fn serve() -> crate::Result<()> {
    let _ = TRANSPORT.set("stdio");
    let started = std::time::Instant::now();
    let outcome = serve_stdio();
    crate::telemetry::record_mcp_session(
        "stdio",
        if outcome.is_ok() { "clean" } else { "error" },
        started.elapsed(),
    );
    outcome
}

/// The stdio request loop, which ends at EOF on stdin.
fn serve_stdio() -> crate::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut reader = std::io::BufReader::new(stdin.lock());
    while let Some(line) = read_request_line(&mut reader, MAX_STDIO_LINE)? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_request(&line) {
            use std::io::Write as _;
            writeln!(out, "{response}")?;
            out.flush()?;
        }
    }
    use std::io::Write as _;
    out.flush()?;
    Ok(())
}

/// Read one newline-terminated request, refusing lines past `limit` instead
/// of allocating the rest of the input.
fn read_request_line(
    reader: &mut impl std::io::BufRead,
    limit: usize,
) -> crate::Result<Option<String>> {
    let mut line = String::new();
    let mut limited = std::io::Read::take(&mut *reader, limit as u64 + 1);
    let read = limited.read_line(&mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if read > limit {
        return Err(format!("MCP request line exceeds the {limit}-byte limit").into());
    }
    Ok(Some(line))
}

// ---------------------------------------------------------------------------
// HTTP transport (opt-in via `leadline mcp --port [N] [--host ADDR]`)
// ---------------------------------------------------------------------------

/// Port used when `--port` is passed without a value.
pub const DEFAULT_HTTP_PORT: u16 = 3000;

/// Bind address used when `--host` is omitted (loopback only).
pub const DEFAULT_HTTP_HOST: &str = "127.0.0.1";

/// Largest JSON-RPC body accepted over HTTP (32 MiB); larger reads fail
/// with 413 instead of growing a buffer without bound.
const MAX_HTTP_BODY: usize = 32 * 1024 * 1024;

/// Largest stdio request line accepted, matching the HTTP body cap.
const MAX_STDIO_LINE: usize = MAX_HTTP_BODY;

/// Largest request or header line accepted (8 KiB).
const MAX_HTTP_LINE: usize = 8 * 1024;

/// Largest header block accepted (64 KiB), counting every line.
const MAX_HTTP_HEADERS: usize = 64 * 1024;

/// Read deadline covering one request's request line and headers.
const HTTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Write deadline for one response.
const HTTP_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Time an answered connection may spend draining the rest of its request
/// before the socket is dropped.
const HTTP_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(100);

/// Concurrent HTTP connections; excess connections fail fast with 503.
const MAX_HTTP_CONNECTIONS: usize = 64;

/// Aggregate request-body bytes buffered across all connections. Without a
/// global ceiling, 64 clients could each declare 32 MiB and stream the full
/// amount inside the read deadline; excess reservations fail fast with 503.
const MAX_INFLIGHT_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Reserved request-body bytes currently buffered by all connections.
static INFLIGHT_BODY_BYTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Parsed `mcp` transport options. `None` means plain stdio ([`serve`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpOptions {
    pub host: String,
    pub port: u16,
}

/// Parse `mcp` arguments: `--port [N]` enables HTTP mode (a bare `--port`
/// means [`DEFAULT_HTTP_PORT`], `0` asks the OS for a free port) and
/// `--host ADDR` sets the bind address (default loopback).
pub fn parse_mcp_args(args: &[String]) -> Result<Option<HttpOptions>, String> {
    let mut host: Option<String> = None;
    let mut port: Option<u16> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--port" => match args.get(index + 1).map(String::as_str) {
                Some(next) if !next.starts_with('-') => {
                    port = Some(parse_port(next)?);
                    index += 1;
                }
                _ => port = Some(DEFAULT_HTTP_PORT),
            },
            "--host" => {
                index += 1;
                host = Some(
                    args.get(index)
                        .cloned()
                        .ok_or_else(|| "--host requires an address".to_owned())?,
                );
            }
            other => return Err(format!("unknown mcp option '{other}'")),
        }
        index += 1;
    }
    match port {
        Some(port) => Ok(Some(HttpOptions {
            host: host.unwrap_or_else(|| DEFAULT_HTTP_HOST.to_owned()),
            port,
        })),
        // `--host` only configures the HTTP transport; silently ignoring it
        // would start stdio with a flag the caller believes is active.
        None if host.is_some() => Err("--host requires --port".to_owned()),
        None => Ok(None),
    }
}

fn parse_port(raw: &str) -> Result<u16, String> {
    raw.parse::<u16>()
        .map_err(|_| format!("invalid --port '{raw}': expected 0-65535"))
}

/// Bind for HTTP mode, falling back to an OS-assigned free port when the
/// requested one is taken. Returns the listener, the actual port, and
/// whether fallback happened (so callers can report it).
pub fn bind_http(host: &str, port: u16) -> std::io::Result<(std::net::TcpListener, u16, bool)> {
    match std::net::TcpListener::bind(format!("{host}:{port}")) {
        Ok(listener) => {
            let actual = listener.local_addr()?.port();
            Ok((listener, actual, false))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse && port != 0 => {
            let listener = std::net::TcpListener::bind(format!("{host}:0"))?;
            let actual = listener.local_addr()?.port();
            Ok((listener, actual, true))
        }
        Err(error) => Err(error),
    }
}

/// Serve the same JSON-RPC API over HTTP: `POST /mcp` takes a request
/// object (or batch array) as the body, and `GET /health` reports status
/// for observability. Diagnostics go to stderr, including the actual port
/// when a taken port falls back to a free one.
pub fn serve_http(host: &str, port: u16) -> crate::Result<()> {
    let (listener, actual, fell_back) = bind_http(host, port)?;
    if fell_back {
        eprintln!("leadline: port {port} in use, using free port {actual}");
    }
    eprintln!("leadline MCP listening on http://{host}:{actual}/mcp");
    serve_listener(listener);
    Ok(())
}

/// Accept loop for an already-bound listener: one worker per connection up
/// to a fixed ceiling, then fail fast with 503 instead of growing threads.
pub fn serve_listener(listener: std::net::TcpListener) {
    let _ = TRANSPORT.set("http");
    let started = std::time::Instant::now();
    let limiter = std::sync::Arc::new(ConnectionLimiter::default());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => match limiter.try_acquire() {
                Some(permit) => {
                    std::thread::spawn(move || {
                        handle_connection(stream);
                        drop(permit);
                    });
                }
                None => {
                    crate::telemetry::record_mcp_error("http", "http_busy");
                    write_response(stream, 503, &http_error("too many concurrent connections"));
                }
            },
            Err(error) => eprintln!("leadline: MCP connection failed: {error}"),
        }
    }
    crate::telemetry::record_mcp_session("http", "clean", started.elapsed());
}

/// Open-connection counter with a fixed ceiling.
#[derive(Default)]
struct ConnectionLimiter {
    open: std::sync::atomic::AtomicUsize,
}

impl ConnectionLimiter {
    fn try_acquire(self: &std::sync::Arc<Self>) -> Option<ConnectionPermit> {
        use std::sync::atomic::Ordering;
        let mut current = self.open.load(Ordering::Relaxed);
        loop {
            if current >= MAX_HTTP_CONNECTIONS {
                return None;
            }
            match self.open.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(ConnectionPermit(std::sync::Arc::clone(self))),
                Err(actual) => current = actual,
            }
        }
    }
}

/// Releases one connection slot on drop.
struct ConnectionPermit(std::sync::Arc<ConnectionLimiter>);

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.0.open.fetch_sub(1, Ordering::AcqRel);
    }
}

/// RAII reservation of buffered request-body bytes.
#[derive(Debug)]
struct BodyReservation(usize);

impl Drop for BodyReservation {
    fn drop(&mut self) {
        INFLIGHT_BODY_BYTES.fetch_sub(self.0, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Reserve `bytes` of buffered request bodies, or fail fast with 503.
fn reserve_body_bytes(bytes: usize) -> Result<BodyReservation, (u16, String)> {
    use std::sync::atomic::Ordering;
    if bytes == 0 {
        return Ok(BodyReservation(0));
    }
    let mut current = INFLIGHT_BODY_BYTES.load(Ordering::Relaxed);
    loop {
        let Some(next) = current.checked_add(bytes) else {
            return Err((503, http_error("request body budget exhausted")));
        };
        if next > MAX_INFLIGHT_BODY_BYTES {
            return Err((503, http_error("request body budget exhausted")));
        }
        match INFLIGHT_BODY_BYTES.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(BodyReservation(bytes)),
            Err(actual) => current = actual,
        }
    }
}

fn handle_connection(stream: std::net::TcpStream) {
    // Loopback-bound listeners only answer loopback `Host` authorities; a
    // wildcard bind answers whatever address the client actually reached.
    // A dual-stack listener reports IPv4-mapped peers (`::ffff:127.0.0.1`),
    // which `Ipv6Addr::is_loopback` alone misses. An unresolvable local
    // address keeps the stricter loopback rule.
    let loopback_only = stream
        .local_addr()
        .map(|address| is_loopback_ip(address.ip()))
        .unwrap_or(true);
    // One whole-request deadline: every blocking read is re-armed against it,
    // so a slow client cannot trickle one byte per timeout window forever.
    let deadline = std::time::Instant::now() + HTTP_READ_TIMEOUT;
    let (status, body) = match read_request(&stream, deadline) {
        Ok(request) => route_http(&request, loopback_only),
        Err(response) => response,
    };
    match status {
        // A 503 from this path is a body-budget refusal, already recorded
        // where the reservation failed; the connection-limit 503 is recorded
        // in the accept loop.
        403 => crate::telemetry::record_mcp_error("http", "origin_rejected"),
        400 | 411 | 413 | 431 => {
            crate::telemetry::record_mcp_error("http", "http_bad_request");
        }
        _ => {}
    }
    write_response(stream, status, &body);
}

/// One parsed HTTP request: request line, `Host`/`Origin`, and body.
struct HttpRequest {
    method: String,
    path: String,
    host: Option<String>,
    origin: Option<String>,
    body: Vec<u8>,
    /// Released when the parsed request (and its body) is dropped.
    _body_reservation: BodyReservation,
}

/// Read one `Connection: close` request: request line, headers, then exactly
/// `Content-Length` body bytes. Lines are bounded, duplicate framing headers
/// and unsupported transfer codings are rejected, and every read is re-armed
/// against the request deadline, so a stalled or endless stream cannot hold
/// a worker open. Body memory is additionally reserved against a process-wide
/// ceiling, so many declared bodies cannot multiply into unbounded buffering.
/// Errors are ready-made status/body pairs.
fn read_request(
    stream: &std::net::TcpStream,
    deadline: std::time::Instant,
) -> Result<HttpRequest, (u16, String)> {
    use std::io::Read as _;
    let mut reader = std::io::BufReader::new(stream);
    let request_line = read_line(&mut reader, MAX_HTTP_LINE, deadline)?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| bad_request("malformed request line"))?
        .to_owned();
    let path = parts
        .next()
        .ok_or_else(|| bad_request("malformed request line"))?
        .to_owned();
    let version = parts
        .next()
        .ok_or_else(|| bad_request("malformed request line"))?;
    if parts.next().is_some() {
        return Err(bad_request("malformed request line"));
    }
    if version != "HTTP/1.1" {
        return Err((505, http_error("HTTP/1.1 required")));
    }
    let mut content_length: Option<usize> = None;
    let mut host = None;
    let mut origin = None;
    let mut header_bytes = 0usize;
    loop {
        let line = read_line(&mut reader, MAX_HTTP_LINE, deadline)?;
        header_bytes += line.len();
        if header_bytes > MAX_HTTP_HEADERS {
            return Err((431, http_error("request headers too large")));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(bad_request("malformed header line"));
        };
        // RFC 7230 field names are tokens: no whitespace before the colon,
        // no characters a stricter intermediary would parse differently.
        if !is_header_token(name) {
            return Err(bad_request("malformed header name"));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                return Err(bad_request("duplicate Content-Length"));
            }
            let raw = value.trim();
            if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(bad_request("invalid Content-Length"));
            }
            content_length = Some(
                raw.parse()
                    .map_err(|_| bad_request("invalid Content-Length"))?,
            );
        } else if name.eq_ignore_ascii_case("host") {
            if host.is_some() {
                return Err(bad_request("duplicate Host"));
            }
            host = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("origin") {
            origin = Some(value.trim().to_owned());
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err((501, http_error("Transfer-Encoding is not supported")));
        }
    }
    let content_length = content_length.unwrap_or(0);
    if content_length > MAX_HTTP_BODY {
        return Err((413, http_error("request body too large")));
    }
    let reservation = reserve_body_bytes(content_length).inspect_err(|_| {
        crate::telemetry::record_mcp_error("http", "body_budget_exhausted");
    })?;
    // `read_exact` would keep the single timeout for every inner read, letting
    // a trickling client stretch one body far past the deadline; re-arm per
    // chunk instead.
    let mut body = vec![0u8; content_length];
    let mut filled = 0usize;
    while filled < body.len() {
        arm_read_deadline(stream, deadline)?;
        let read = reader
            .read(&mut body[filled..])
            .map_err(|_| (408, http_error("request body read timed out")))?;
        if read == 0 {
            return Err(bad_request("truncated request body"));
        }
        filled += read;
    }
    Ok(HttpRequest {
        method,
        path,
        host,
        origin,
        body,
        _body_reservation: reservation,
    })
}

/// Re-arm the socket read timeout against one whole-request deadline.
fn arm_read_deadline(
    stream: &std::net::TcpStream,
    deadline: std::time::Instant,
) -> Result<(), (u16, String)> {
    let remaining = deadline
        .checked_duration_since(std::time::Instant::now())
        .ok_or_else(|| (408, http_error("request read deadline exceeded")))?;
    stream
        .set_read_timeout(Some(remaining.max(std::time::Duration::from_millis(1))))
        .map_err(|_| (500, http_error("cannot set read deadline")))?;
    Ok(())
}

/// Read one line up to `limit` bytes (inclusive of `\n`); past it, 431.
fn read_line(
    reader: &mut std::io::BufReader<&std::net::TcpStream>,
    limit: usize,
    deadline: std::time::Instant,
) -> Result<String, (u16, String)> {
    use std::io::BufRead as _;
    let mut bytes = Vec::new();
    loop {
        arm_read_deadline(reader.get_ref(), deadline)?;
        let available = reader
            .fill_buf()
            .map_err(|_| (408, http_error("request read timed out")))?;
        if available.is_empty() {
            // EOF mid-line is a truncated request, not an empty header line.
            if bytes.last().is_none_or(|byte| *byte != b'\n') {
                return Err(bad_request("connection closed before line end"));
            }
            break;
        }
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if bytes.len() + take > limit {
            return Err((431, http_error("request line or header too large")));
        }
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if bytes.last() == Some(&b'\n') {
            break;
        }
    }
    String::from_utf8(bytes).map_err(|_| bad_request("request must be UTF-8"))
}

fn bad_request(message: &str) -> (u16, String) {
    (400, http_error(message))
}

fn http_error(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}

/// RFC 7230 token characters, valid for one header field name.
fn is_header_token(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Route one HTTP request to the shared JSON-RPC handler.
///
/// HTTP/1.1 requires exactly one `Host`; loopback-bound listeners only accept
/// loopback authorities. MCP Streamable HTTP requires `Origin` validation
/// against DNS rebinding: browser origins must be loopback, and anything else
/// is 403. Non-browser clients omit `Origin` and pass.
fn route_http(request: &HttpRequest, loopback_only: bool) -> (u16, String) {
    if !host_allowed(request.host.as_deref(), loopback_only) {
        return (400, http_error("invalid Host header"));
    }
    if !origin_allowed(request.origin.as_deref()) {
        return (403, http_error("origin not allowed"));
    }
    let path = request.path.split('?').next().unwrap_or(&request.path);
    match (request.method.as_str(), path) {
        ("GET", "/health") => (
            200,
            serde_json::json!({
                "status": "ok",
                "transport": "http",
                "version": env!("CARGO_PKG_VERSION"),
                "tools": TOOL_NAMES.len(),
            })
            .to_string(),
        ),
        ("POST", "/mcp" | "/") => match std::str::from_utf8(&request.body) {
            Ok(text) => {
                if text.trim().is_empty() {
                    return bad_request("empty JSON-RPC body");
                }
                match handle_request(text) {
                    // Notifications (no `id`) carry no JSON-RPC response.
                    Some(response) => (200, response),
                    None => (202, String::new()),
                }
            }
            Err(_) => bad_request("request body must be UTF-8 JSON"),
        },
        ("GET", "/mcp") => (405, http_error("use POST /mcp with a JSON-RPC body")),
        _ => (404, http_error("not found: try POST /mcp or GET /health")),
    }
}

/// `true` when the request carries no browser origin or a loopback one.
///
/// Browsers always send an `http(s)://` scheme; scheme-less values (including
/// `null`) and non-loopback authorities are rejected.
fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split('/').next().unwrap_or("");
    loopback_hostname(authority_host(authority))
}

/// `true` for `localhost` and any loopback IP literal, IPv4-mapped included.
fn loopback_hostname(name: &str) -> bool {
    name.eq_ignore_ascii_case("localhost")
        || name.parse::<std::net::IpAddr>().is_ok_and(is_loopback_ip)
}

/// `true` for any loopback address, including IPv4-mapped IPv6 peers.
fn is_loopback_ip(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// Host part of one `host[:port]` authority, IPv6 brackets stripped.
fn authority_host(authority: &str) -> &str {
    let host = match authority.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty() && port.chars().all(|digit| digit.is_ascii_digit()) =>
        {
            host
        }
        _ => authority,
    };
    host.trim_matches(['[', ']'])
}

/// `true` when a request carries the single `Host` HTTP/1.1 requires, and (on
/// a loopback-bound listener) that host is loopback.
fn host_allowed(host: Option<&str>, loopback_only: bool) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = authority_host(host);
    if host.is_empty() {
        return false;
    }
    !loopback_only || loopback_hostname(host)
}

/// `Write` adapter that re-arms the socket write timeout against one
/// whole-response deadline, so a slow reader cannot pin a worker past it.
struct DeadlineWriter<'a> {
    stream: &'a std::net::TcpStream,
    deadline: std::time::Instant,
}

impl std::io::Write for DeadlineWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let remaining = self
            .deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::TimedOut, "response deadline exceeded")
            })?;
        self.stream
            .set_write_timeout(Some(remaining.max(std::time::Duration::from_millis(1))))?;
        (&*self.stream).write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.stream).flush()
    }
}

fn write_response(stream: std::net::TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "OK",
    };
    use std::io::Write as _;
    let stream = stream;
    let mut writer = DeadlineWriter {
        stream: &stream,
        deadline: std::time::Instant::now() + HTTP_WRITE_TIMEOUT,
    };
    let _ = write!(
        writer,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = writer.flush();
    // Half-close, then read off what is left of the request. Closing a socket
    // with unread bytes still queued makes the kernel send RST, and an early
    // reject such as 431 leaves most of the request behind; either can break
    // a peer that is still writing or has not read the response yet.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    drain_request(&stream);
}

/// Discard the rest of a request after the response is sent, bounded by one
/// short deadline so a slow client cannot pin the worker.
fn drain_request(mut stream: &std::net::TcpStream) {
    use std::io::Read as _;
    let deadline = std::time::Instant::now() + HTTP_DRAIN_TIMEOUT;
    let mut scratch = [0u8; 4096];
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        let _ = stream.set_read_timeout(Some(remaining.max(std::time::Duration::from_millis(1))));
        match stream.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
}

/// Handle one raw input line, returning the response line if any.
///
/// Pure function (no I/O beyond analysis reads) so tests can drive it
/// directly without subprocesses. Returns `None` for notifications
/// (requests without an `id`) and for blank input.
pub fn handle_request(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(_) => {
            crate::telemetry::record_mcp_error(transport(), "parse_error");
            return Some(parse_error_response());
        }
    };
    if let serde_json::Value::Array(batch) = value {
        // An empty batch is an invalid request, not a notification.
        if batch.is_empty() {
            crate::telemetry::record_mcp_error(transport(), "invalid_request");
            return Some(
                error_response(serde_json::Value::Null, -32600, "Invalid Request").to_string(),
            );
        }
        // An oversized batch is rejected, but a batch of notifications must
        // still never draw a response.
        if batch.len() > MAX_BATCH_REQUESTS {
            crate::telemetry::record_mcp_error(transport(), "batch_too_large");
            let expects_response = batch.iter().any(|item| item.get("id").is_some());
            return expects_response.then(|| batch_error_response(-32600, "Invalid Request"));
        }
        // Serialize as we go so one batch cannot retain an unbounded number
        // of complete responses before the limit is known.
        let mut parts: Vec<String> = Vec::new();
        let mut total = 0usize;
        for item in &batch {
            if let Some(response) = handle_single(item) {
                let text = response.to_string();
                total += text.len();
                if total > MAX_RESPONSE_BYTES {
                    crate::telemetry::record_mcp_error(transport(), "response_too_large");
                    return Some(batch_error_response(
                        -32603,
                        "batch response exceeds the size limit",
                    ));
                }
                parts.push(text);
            }
        }
        if parts.is_empty() {
            return None;
        }
        // The incremental check omits separators; enforce the exact rendered
        // size before returning it.
        let rendered = format!("[{}]", parts.join(","));
        if rendered.len() > MAX_RESPONSE_BYTES {
            crate::telemetry::record_mcp_error(transport(), "response_too_large");
            return Some(batch_error_response(
                -32603,
                "batch response exceeds the size limit",
            ));
        }
        return Some(rendered);
    }
    let notification = value.get("id").is_none();
    let method = value
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let response = handle_single(&value)?;
    let text = response.to_string();
    crate::telemetry::record_mcp_payload(
        &method,
        notification,
        raw.len() as u64,
        text.len() as u64,
    );
    if text.len() > MAX_RESPONSE_BYTES {
        crate::telemetry::record_mcp_error(transport(), "response_too_large");
        let id = value
            .get("id")
            .cloned()
            .filter(valid_id)
            .unwrap_or(serde_json::Value::Null);
        let fallback = error_response(id, -32603, "response exceeds the size limit").to_string();
        // A huge echoed id can keep the fallback over the cap too; drop it.
        if fallback.len() > MAX_RESPONSE_BYTES {
            return Some(
                error_response(
                    serde_json::Value::Null,
                    -32603,
                    "response exceeds the size limit",
                )
                .to_string(),
            );
        }
        return Some(fallback);
    }
    Some(text)
}

fn handle_single(request: &serde_json::Value) -> Option<serde_json::Value> {
    let object = match request.as_object() {
        Some(object) => object,
        None => {
            return Some(error_response(
                serde_json::Value::Null,
                -32600,
                "Invalid Request",
            ));
        }
    };
    let jsonrpc = object.get("jsonrpc").and_then(serde_json::Value::as_str);
    let method = object.get("method").and_then(serde_json::Value::as_str);
    let id = object.get("id").cloned();
    // A request without `id` is a notification only when it carries a valid
    // method; anything else is an invalid request that still gets a response.
    if jsonrpc != Some("2.0") || method.is_none() {
        let id = id.filter(valid_id).unwrap_or(serde_json::Value::Null);
        return Some(error_response(id, -32600, "Invalid Request"));
    }
    let method = method.unwrap_or_default().to_owned();
    crate::telemetry::record_mcp_method(&method, id.is_none());
    if let Some(id) = &id
        && !valid_id(id)
    {
        return Some(error_response(
            serde_json::Value::Null,
            -32600,
            "Invalid Request",
        ));
    }
    // Params, when present, must be structured; `null` is accepted as
    // equivalent to omission for hosts that serialize optional params that
    // way, but strings, numbers, and booleans are rejected.
    let params = object
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if !params.is_null() && !params.is_object() && !params.is_array() {
        return id.map(|id| error_response(id, -32602, "Invalid params"));
    }
    let Some(id) = id else {
        // Notifications run for their side effects and never get a response;
        // this server is stateless, so only the dispatch matters.
        if method != "initialized" && !method.starts_with("notifications/") {
            let _ = dispatch_method(&method, &params);
        }
        return None;
    };
    // `initialized` and `notifications/*` are acknowledgements.
    if method == "initialized" || method.starts_with("notifications/") {
        return Some(success_response(id, serde_json::json!({})));
    }
    match dispatch_method(&method, &params) {
        Ok(result) => Some(success_response(id, result)),
        Err((code, message)) => {
            if code == -32601 {
                crate::telemetry::record_mcp_error(transport(), "method_not_found");
            }
            Some(error_response(id, code, &message))
        }
    }
}

/// Dispatch one JSON-RPC method, returning its unwrapped result.
fn dispatch_method(
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, (i64, String)> {
    match method {
        "initialize" => Ok(initialize_result(params)),
        "ping" => Ok(serde_json::json!({})),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => dispatch_tools_call(params)
            .map(tool_result)
            .inspect_err(|_| {
                crate::telemetry::record_mcp_error(transport(), "tool_error");
            }),
        name if TOOL_NAMES.contains(&name) => {
            // Direct tool methods are an extension; they take named parameters
            // only, so an array must not silently fall back to default paths.
            if params.is_array() {
                return Err((-32602, "tool parameters must be an object".to_owned()));
            }
            dispatch_tool(name, params)
                .map(tool_result)
                .inspect_err(|_| {
                    crate::telemetry::record_mcp_error(transport(), "tool_error");
                })
        }
        _ => Err((-32601, "Method not found".to_owned())),
    }
}

/// JSON-RPC ids are strings, numbers, or null; structured ids are invalid.
fn valid_id(id: &serde_json::Value) -> bool {
    matches!(
        id,
        serde_json::Value::String(_) | serde_json::Value::Number(_) | serde_json::Value::Null
    )
}

/// Wrap a tool payload in the MCP `CallToolResult` shape: a text content
/// block for hosts that render text, plus `structuredContent` so code-mode
/// style hosts can hand agents native JSON. Clients that receive neither
/// surface a null result, so this envelope is required, not cosmetic.
fn tool_result(payload: serde_json::Value) -> serde_json::Value {
    let text = match &payload {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": payload,
    })
}

fn dispatch_tools_call(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    let object = params
        .as_object()
        .ok_or((-32602, "tools/call requires {name, arguments}".to_owned()))?;
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or((-32602, "tools/call requires a string name".to_owned()))?;
    if !TOOL_NAMES.contains(&name) {
        return Err((-32602, format!("unknown tool '{name}'")));
    }
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    dispatch_tool(name, &arguments)
}

fn dispatch_tool(
    name: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, (i64, String)> {
    let started = std::time::Instant::now();
    let _inflight = Inflight::enter(transport());
    let sampler = crate::telemetry::Sampler::start("mcp", name);
    let result = dispatch_tool_inner(name, params);
    let outcome = match &result {
        Ok(payload)
            if name == "check"
                && payload.get("passed") == Some(&serde_json::Value::Bool(false)) =>
        {
            "gate_failed"
        }
        Ok(_) => "success",
        Err((code, _)) if *code == -32602 => "usage_error",
        Err(_) => "internal_error",
    };
    let cost = sampler.finish();
    crate::telemetry::record_process_cost("mcp", name, outcome, &cost);
    crate::telemetry::record_invocation("mcp", name, outcome, started.elapsed());
    result
}

/// The transport this process serves, set once when a server starts.
static TRANSPORT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// Transport label for this process; a direct `handle_request` call is a
/// stdio-style session.
fn transport() -> &'static str {
    TRANSPORT.get().copied().unwrap_or("stdio")
}

/// Tool calls currently executing, reported as a gauge on entry and exit so an
/// early return cannot leave it stuck.
static INFLIGHT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct Inflight(&'static str);

impl Inflight {
    fn enter(transport: &'static str) -> Self {
        let count = INFLIGHT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        crate::telemetry::set_mcp_inflight(transport, count);
        Self(transport)
    }
}

impl Drop for Inflight {
    fn drop(&mut self) {
        let count = INFLIGHT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) - 1;
        crate::telemetry::set_mcp_inflight(self.0, count);
    }
}

fn dispatch_tool_inner(
    name: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, (i64, String)> {
    let params = if params.is_null() {
        &serde_json::Value::Object(serde_json::Map::new())
    } else {
        params
    };
    match name {
        "analyze" => tool_analyze(params),
        "analyze_changed" => tool_analyze_changed(params),
        "analyze_function" => tool_analyze_function(params),
        "check" => tool_check(params),
        "explain_metric" => tool_explain_metric(params),
        "repo_summary" => tool_repo_summary(params),
        "sql_plan" => tool_sql_plan(params),
        "security_findings" => tool_security_findings(params),
        "vulnerabilities" => tool_vulnerabilities(params),
        "sql_risks" => tool_sql_risks(params),
        "test_targets" => tool_test_targets(params),
        "dependencies" => tool_dependencies(params),
        "impact" => tool_impact(params),
        "coupling" => tool_coupling(params),
        "hotspots" => tool_hotspots(params),
        "duplication" => tool_duplication(params),
        "policy" => tool_policy(params),
        "risk" => tool_risk(params),
        "debt" => tool_debt(params),
        "project" => tool_project(params),
        _ => Err((-32602, format!("unknown tool '{name}'"))),
    }
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

fn metric_specs() -> Result<serde_json::Value, (i64, String)> {
    serde_json::to_value(crate::core::MetricSpecs::default())
        .map_err(|error| (-32603, format!("internal error: {error}")))
}

/// Every tool result carries the version envelope so agents can pin behavior.
fn envelope(
    mut fields: serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, (i64, String)> {
    fields.insert(
        "schema_version".to_owned(),
        serde_json::Value::from(OUTPUT_SCHEMA_VERSION),
    );
    fields.insert(
        "analyzer_version".to_owned(),
        serde_json::Value::from(env!("CARGO_PKG_VERSION")),
    );
    fields.insert("metric_specs".to_owned(), metric_specs()?);
    Ok(serde_json::Value::Object(fields))
}

/// Serialize a report struct into an envelope object.
fn report_object<T: serde::Serialize>(
    report: &T,
) -> Result<serde_json::Map<String, serde_json::Value>, (i64, String)> {
    match serde_json::to_value(report).map_err(|error| (-32603, error.to_string()))? {
        serde_json::Value::Object(fields) => Ok(fields),
        _ => Err((-32603, "report did not serialize to an object".to_owned())),
    }
}

/// Copy named agent-json fields into envelope fields. The envelope's own
/// version fields overwrite the report's duplicates afterwards.
fn merge_agent_fields(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    agent: &serde_json::Value,
    keys: &[&str],
) {
    for key in keys {
        if let Some(value) = agent.get(key) {
            fields.insert((*key).to_owned(), value.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Compact rows
// ---------------------------------------------------------------------------

/// Trimmed agent-facing function row: identity + spans + the metrics an
/// agent acts on. Intentionally mirrors the agent-json field set.
fn compact_function(path: &str, function: &FunctionAnalysis) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("name".to_owned(), serde_json::json!(function.name));
    fields.insert("line".to_owned(), serde_json::json!(function.start_line));
    fields.insert(
        "start_line".to_owned(),
        serde_json::json!(function.start_line),
    );
    fields.insert("end_line".to_owned(), serde_json::json!(function.end_line));
    fields.extend(metric_fields(&function.metrics));
    serde_json::Value::Object(fields)
}

/// `check` violation row: a compact function plus the reasons it failed.
fn checked_function(
    path: &str,
    function: &FunctionAnalysis,
    reasons: Vec<&'static str>,
) -> serde_json::Value {
    let mut row = compact_function(path, function);
    if let serde_json::Value::Object(fields) = &mut row {
        fields.insert("reason".to_owned(), serde_json::json!(reasons));
    }
    row
}

fn compact_metrics(value: &FunctionAnalysis) -> serde_json::Value {
    let mut fields = serde_json::Map::new();
    fields.insert("line".to_owned(), serde_json::json!(value.start_line));
    fields.extend(metric_fields(&value.metrics));
    serde_json::Value::Object(fields)
}

/// The shared metric field set both compact rows carry.
fn metric_fields(metrics: &FunctionMetrics) -> serde_json::Map<String, serde_json::Value> {
    let mut fields = serde_json::Map::new();
    fields.insert("cognitive".to_owned(), serde_json::json!(metrics.cognitive));
    fields.insert(
        "cyclomatic".to_owned(),
        serde_json::json!(metrics.cyclomatic),
    );
    fields.insert("crap".to_owned(), round1_opt(metrics.crap));
    fields.insert("coverage".to_owned(), round1_opt(metrics.coverage));
    fields
}

fn round1_opt(value: Option<f64>) -> serde_json::Value {
    match value {
        Some(number) if number.is_finite() => {
            serde_json::json!((number * 10.0).round() / 10.0)
        }
        _ => serde_json::Value::Null,
    }
}

/// Cap a result array at `limit`. Returns `(kept, truncated, total)`.
fn cap_with_limit(
    entries: &mut Vec<serde_json::Value>,
    limit: usize,
) -> (Vec<serde_json::Value>, bool, usize) {
    let total = entries.len();
    if total > limit {
        (entries[..limit].to_vec(), true, total)
    } else {
        (std::mem::take(entries), false, total)
    }
}

/// Cap gate violations at the same entry budget as findings. Returns the
/// serialized rows plus whether anything was dropped.
fn cap_violations<T: serde::Serialize>(
    violations: &[T],
    limit: usize,
) -> Result<(serde_json::Value, bool), (i64, String)> {
    let truncated = violations.len() > limit;
    let rows = violations
        .iter()
        .take(limit)
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| (-32603, error.to_string()))?;
    Ok((serde_json::Value::Array(rows), truncated))
}

/// Output budget for list-shaped tools, mirroring the CLI agent-json flags.
#[derive(Clone, Copy, Debug, Default)]
struct ToolBudget {
    top: Option<usize>,
    sort_by: Option<SortKey>,
    min_crap: Option<f64>,
    min_delta: Option<f64>,
}

impl ToolBudget {
    fn limit(&self) -> usize {
        self.top.unwrap_or(MAX_ENTRIES)
    }
}

fn parse_tool_budget(
    params: &serde_json::Value,
    allow_min_delta: bool,
) -> Result<ToolBudget, (i64, String)> {
    let top = match params.get("top") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let count = value
                .as_u64()
                .ok_or((-32602, "top must be a positive integer".to_owned()))?;
            let count =
                usize::try_from(count).map_err(|_| (-32602, "top must fit in usize".to_owned()))?;
            if count == 0 {
                return Err((-32602, "top must be at least 1".to_owned()));
            }
            if count > MAX_ENTRIES {
                return Err((-32602, format!("top must be at most {MAX_ENTRIES}")));
            }
            Some(count)
        }
    };
    let sort_by = match opt_str(params, "sort_by")? {
        None => None,
        Some(value) => Some(SortKey::parse(value).ok_or((
            -32602,
            format!("unknown sort_by '{value}': expected 'crap', 'cognitive', or 'cyclomatic'"),
        ))?),
    };
    let min_crap = parse_optional_f64(params, "min_crap")?;
    let min_delta = if allow_min_delta {
        parse_optional_f64(params, "min_delta")?
    } else {
        None
    };
    Ok(ToolBudget {
        top,
        sort_by,
        min_crap,
        min_delta,
    })
}

fn parse_optional_f64(params: &serde_json::Value, key: &str) -> Result<Option<f64>, (i64, String)> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let number = value
                .as_f64()
                .ok_or((-32602, format!("{key} must be a number")))?;
            if !number.is_finite() {
                return Err((-32602, format!("{key} must be finite")));
            }
            Ok(Some(number))
        }
    }
}

fn row_u32(row: &serde_json::Value, key: &str) -> u32 {
    row.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32
}

fn row_f64(row: &serde_json::Value, key: &str) -> Option<f64> {
    row.get(key).and_then(serde_json::Value::as_f64)
}

/// Descending order by metric; missing values sort last.
fn cmp_desc_f64(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (Some(left), Some(right)) => right.total_cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn row_position(row: &serde_json::Value) -> (&str, &str, u32) {
    (
        row_str(row, "path"),
        row_str(row, "name"),
        row_u32(row, "line"),
    )
}

/// Sort compact `analyze` rows: primary metric descending (missing CRAP
/// last), then path/name/line for determinism.
fn sort_analyze_rows(rows: &mut [serde_json::Value], key: SortKey) {
    rows.sort_by(|left, right| {
        let primary = match key {
            SortKey::Crap => cmp_desc_f64(row_f64(left, "crap"), row_f64(right, "crap")),
            SortKey::Cognitive => row_u32(right, "cognitive").cmp(&row_u32(left, "cognitive")),
            SortKey::Cyclomatic => row_u32(right, "cyclomatic").cmp(&row_u32(left, "cyclomatic")),
        };
        primary.then_with(|| row_position(left).cmp(&row_position(right)))
    });
}

/// The current-side metrics of a changed row: after when present, else before.
fn changed_current(row: &serde_json::Value) -> &serde_json::Value {
    row.get("after")
        .filter(|value| !value.is_null())
        .or_else(|| row.get("before"))
        .unwrap_or(row)
}

/// Max absolute delta across cognitive, cyclomatic, and CRAP; 0.0 when the
/// row pairs no numeric delta (added or removed functions).
fn changed_max_delta(row: &serde_json::Value) -> f64 {
    let delta = row.get("delta");
    ["cognitive", "cyclomatic", "crap"]
        .iter()
        .filter_map(|key| {
            delta
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_f64)
        })
        .map(f64::abs)
        .fold(0.0, f64::max)
}

/// Sort changed rows by current-side metric descending.
fn sort_changed_rows(rows: &mut [serde_json::Value], key: SortKey) {
    rows.sort_by(|left, right| {
        let left_metric = changed_current(left);
        let right_metric = changed_current(right);
        let primary = match key {
            SortKey::Crap => {
                cmp_desc_f64(row_f64(left_metric, "crap"), row_f64(right_metric, "crap"))
            }
            SortKey::Cognitive => {
                row_u32(right_metric, "cognitive").cmp(&row_u32(left_metric, "cognitive"))
            }
            SortKey::Cyclomatic => {
                row_u32(right_metric, "cyclomatic").cmp(&row_u32(left_metric, "cyclomatic"))
            }
        };
        primary.then_with(|| row_position(left).cmp(&row_position(right)))
    });
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn tool_analyze(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "analyze",
        &["path", "coverage", "top", "sort_by", "min_crap", "index"],
    )?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let budget = parse_tool_budget(params, false)?;
    let coverage_path = opt_str(params, "coverage")?;
    let coverage = coverage_path
        .map(load_coverage_file)
        .transpose()
        .map_err(|message| (-32602, message))?;
    let config = load_config(path)?;
    let excludes = config_excludes(config.as_ref());
    let (report, reuse) =
        analyze_read_only(params, config.as_ref(), path, coverage.as_ref(), &excludes)?;
    let mut rows = Vec::new();
    for file in &report.files {
        for function in &file.functions {
            rows.push(compact_function(&file.path, function));
        }
    }
    if let Some(floor) = budget.min_crap {
        rows.retain(|row| row_f64(row, "crap").is_some_and(|score| score >= floor));
    }
    if let Some(key) = budget.sort_by {
        sort_analyze_rows(&mut rows, key);
    }
    let (functions, truncated, total) = cap_with_limit(&mut rows, budget.limit());
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("analyze"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("functions".to_owned(), serde_json::Value::Array(functions));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    if let Some(counts) = index_counts(reuse) {
        fields.insert("index".to_owned(), counts);
    }
    envelope(fields)
}

fn tool_analyze_changed(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "analyze_changed",
        &[
            "base",
            "path",
            "target",
            "renames",
            "explain",
            "top",
            "sort_by",
            "min_crap",
            "min_delta",
        ],
    )?;
    let base = opt_str(params, "base")?.unwrap_or("HEAD~1");
    let path = opt_str(params, "path")?.unwrap_or(".");
    let target = match opt_str(params, "target")? {
        None | Some("worktree") => crate::diff::ComparisonTarget::Worktree,
        Some("index") => crate::diff::ComparisonTarget::Index,
        Some(revision) => crate::diff::ComparisonTarget::Revision(revision.to_owned()),
    };
    let detect_renames = opt_flag(params, "renames", "analyze_changed")?;
    let explain = opt_flag(params, "explain", "analyze_changed")?;
    let budget = parse_tool_budget(params, true)?;
    let options = crate::diff::ChangeOptions {
        base: base.to_owned(),
        target,
        detect_renames,
    };
    let report = crate::diff::analyze_changes(Path::new(path), &options)
        .map_err(|error| (-32602, error.to_string()))?;
    let mut rows = Vec::new();
    for change in &report.functions {
        let line = change
            .after
            .as_ref()
            .or(change.before.as_ref())
            .map_or(0, |function| function.start_line);
        let (start_line, end_line) = change
            .after
            .as_ref()
            .or(change.before.as_ref())
            .map_or((line, line), |function| {
                (function.start_line, function.end_line)
            });
        let mut row = serde_json::json!({
            "path": change.path,
            "name": change.name,
            "line": line,
            "start_line": start_line,
            "end_line": end_line,
            "before": change.before.as_ref().map(compact_metrics),
            "after": change.after.as_ref().map(compact_metrics),
            "delta": change_delta(change.before.as_ref(), change.after.as_ref()),
        });
        if explain
            && let Some(causes) = changed_causes(change.before.as_ref(), change.after.as_ref())
        {
            row["causes"] = causes;
        }
        rows.push(row);
    }
    if let Some(floor) = budget.min_crap {
        rows.retain(|row| {
            row_f64(changed_current(row), "crap").is_some_and(|score| score >= floor)
        });
    }
    if let Some(floor) = budget.min_delta {
        rows.retain(|row| changed_max_delta(row) >= floor);
    }
    if let Some(key) = budget.sort_by {
        sort_changed_rows(&mut rows, key);
    }
    let (changes, truncated, total) = cap_with_limit(&mut rows, budget.limit());
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("analyze_changed"));
    fields.insert("base".to_owned(), serde_json::json!(report.base));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("changes".to_owned(), serde_json::Value::Array(changes));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

fn change_delta(
    before: Option<&FunctionAnalysis>,
    after: Option<&FunctionAnalysis>,
) -> serde_json::Value {
    match (before, after) {
        (Some(left), Some(right)) => {
            let crap = match (left.metrics.crap, right.metrics.crap) {
                (Some(a), Some(b)) if a.is_finite() && b.is_finite() => {
                    serde_json::json!(((b - a) * 10.0).round() / 10.0)
                }
                _ => serde_json::Value::Null,
            };
            serde_json::json!({
                "cognitive": right.metrics.cognitive as i64 - left.metrics.cognitive as i64,
                "cyclomatic": right.metrics.cyclomatic as i64 - left.metrics.cyclomatic as i64,
                "crap": crap,
            })
        }
        _ => serde_json::Value::Null,
    }
}

fn tool_analyze_function(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "analyze_function", &["path", "function", "explain"])?;
    let path = req_str(params, "path")?;
    let function = req_str(params, "function")?;
    let file = PathBuf::from(path);
    let root = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut analyzed =
        crate::analyze_file(&file, &root, None).map_err(|error| (-32602, error.to_string()))?;
    analyzed.path = crate::normalize_path(&file);
    analyzed
        .functions
        .retain(|candidate| candidate.name == function);
    if analyzed.functions.is_empty() {
        return Err((
            -32602,
            format!("function '{function}' was not found in {path}"),
        ));
    }
    let mut rows: Vec<serde_json::Value> = analyzed
        .functions
        .iter()
        .map(|item| compact_function(&analyzed.path, item))
        .collect();
    let explain = opt_flag(params, "explain", "analyze_function")?;
    if explain {
        for (row, function) in rows.iter_mut().zip(&analyzed.functions) {
            row["contributions"] =
                serde_json::to_value(&function.contributions).unwrap_or(serde_json::Value::Null);
        }
    }
    let total = rows.len();
    let (functions, truncated, _) = cap_with_limit(&mut rows, MAX_ENTRIES);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("analyze_function"));
    fields.insert("path".to_owned(), serde_json::json!(analyzed.path));
    fields.insert("function".to_owned(), serde_json::json!(function));
    fields.insert("functions".to_owned(), serde_json::Value::Array(functions));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

fn tool_check(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "check",
        &[
            "path",
            "base",
            "baseline",
            "coverage",
            "index",
            "thresholds",
            "regressions",
        ],
    )?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let config = load_config(path)?;
    let regression_limits = parse_regression_limits(params, config.as_ref())?;
    let base = opt_str(params, "base")?;
    let baseline_path = opt_str(params, "baseline")?;
    if base.is_some() && baseline_path.is_some() {
        return Err((-32602, "base and baseline are exclusive".to_owned()));
    }
    if regression_limits.is_some() && base.is_none() && baseline_path.is_none() {
        return Err((
            -32602,
            "check regressions requires a base revision or baseline file".to_owned(),
        ));
    }
    let mut thresholds = parse_thresholds(params, regression_limits.is_some())?;
    if let Some(config) = config.as_ref() {
        fill_thresholds(&mut thresholds, config);
    }
    let coverage = opt_str(params, "coverage")?
        .map(load_coverage_file)
        .transpose()
        .map_err(|message| (-32602, message))?;
    let excludes = config_excludes(config.as_ref());
    let mut rows = Vec::new();
    let mut reuse = None;
    // Parse-error counts per language for metrics; paths never leave.
    let mut parse_languages: Vec<(&'static str, u64)> = Vec::new();
    let mut note_parse_errors = |language: &'static str, count: u64| {
        if count == 0 {
            return;
        }
        match parse_languages
            .iter_mut()
            .find(|(known, _)| *known == language)
        {
            Some(slot) => slot.1 += count,
            None => parse_languages.push((language, count)),
        }
    };
    if let Some(base) = base {
        let mut report = crate::diff::analyze_changed(Path::new(path), base)
            .map_err(|error| (-32602, error.to_string()))?;
        if let Some(coverage) = coverage.as_ref() {
            for change in &mut report.functions {
                if let Some(before) = change.before.as_mut() {
                    coverage.apply_function(&change.path, before);
                }
                if let Some(after) = change.after.as_mut() {
                    coverage.apply_function(&change.path, after);
                }
            }
        }
        for change in &report.functions {
            let Some(after) = change.after.as_ref() else {
                continue;
            };
            let mut reasons = thresholds.violation_reasons(&after.metrics);
            if regression_limits.as_ref().is_some_and(|limits| {
                change
                    .before
                    .as_ref()
                    .is_some_and(|before| crate::diff::regression_violates(before, after, limits))
            }) {
                reasons.push("regression");
            }
            if !reasons.is_empty() {
                rows.push(checked_function(&change.path, after, reasons));
            }
        }
        for diagnostics in &report.parse_errors {
            if let Some(language) = crate::telemetry::language_label(&diagnostics.path) {
                note_parse_errors(language, diagnostics.after.len() as u64);
            }
        }
    } else if let Some(baseline_path) = baseline_path {
        let baseline = crate::baseline::Baseline::read(Path::new(baseline_path))
            .map_err(|error| (-32602, error.to_string()))?;
        let (report, warm) =
            analyze_read_only(params, config.as_ref(), path, coverage.as_ref(), &excludes)?;
        reuse = warm;
        let regressed: BTreeSet<(String, String)> = match &regression_limits {
            Some(limits) => baseline
                .compare(&report, limits)
                .iter()
                .map(|finding| (finding.path.clone(), finding.id.clone()))
                .collect(),
            None => BTreeSet::new(),
        };
        for file in &report.files {
            for function in &file.functions {
                let mut reasons = thresholds.violation_reasons(&function.metrics);
                if regression_limits.is_some()
                    && regressed.contains(&(file.path.clone(), function.id.clone()))
                {
                    reasons.push("regression");
                }
                if !reasons.is_empty() {
                    rows.push(checked_function(&file.path, function, reasons));
                }
            }
        }
        for file in &report.files {
            note_parse_errors(file.language.as_str(), file.parse_errors.len() as u64);
        }
    } else {
        let (report, warm) =
            analyze_read_only(params, config.as_ref(), path, coverage.as_ref(), &excludes)?;
        reuse = warm;
        for file in &report.files {
            for function in &file.functions {
                let reasons = thresholds.violation_reasons(&function.metrics);
                if !reasons.is_empty() {
                    rows.push(checked_function(&file.path, function, reasons));
                }
            }
        }
        for file in &report.files {
            note_parse_errors(file.language.as_str(), file.parse_errors.len() as u64);
        }
    }
    let passed = rows.is_empty();
    let (violations, truncated, total) = cap_with_limit(&mut rows, MAX_ENTRIES);
    // MCP check runs no scanner gates, so only function and parse-error
    // findings are recorded.
    crate::telemetry::record_check_findings(
        "mcp",
        &crate::telemetry::CheckFindings {
            functions: total as u64,
            parse_errors: parse_languages.iter().map(|(_, count)| count).sum(),
            security: 0,
            vulnerabilities: 0,
            sql: 0,
            severity: &[],
            languages: &parse_languages,
        },
    );
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("check"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    if let Some(base) = base {
        fields.insert("base".to_owned(), serde_json::json!(base));
    }
    if let Some(baseline_path) = baseline_path {
        fields.insert("baseline".to_owned(), serde_json::json!(baseline_path));
    }
    fields.insert(
        "thresholds".to_owned(),
        serde_json::json!({
            "cognitive": thresholds.cognitive,
            "cyclomatic": thresholds.cyclomatic,
            "crap": thresholds.crap,
            "max_nesting": thresholds.max_nesting,
        }),
    );
    fields.insert(
        "regressions".to_owned(),
        serde_json::json!(regression_limits.as_ref().map(|limits| {
            serde_json::json!({
                "cognitive": limits.cognitive,
                "cyclomatic": limits.cyclomatic,
                "crap": limits.crap,
                "max_nesting": limits.max_nesting,
            })
        })),
    );
    fields.insert("passed".to_owned(), serde_json::Value::from(passed));
    fields.insert(
        "violations".to_owned(),
        serde_json::Value::Array(violations),
    );
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    if let Some(counts) = index_counts(reuse) {
        fields.insert("index".to_owned(), counts);
    }
    envelope(fields)
}

/// Fill request gaps from `[thresholds]`; explicit MCP fields win.
fn fill_thresholds(thresholds: &mut Thresholds, config: &crate::config::Config) {
    if thresholds.cognitive.is_none() {
        thresholds.cognitive = config.thresholds.cognitive;
    }
    if thresholds.cyclomatic.is_none() {
        thresholds.cyclomatic = config.thresholds.cyclomatic;
    }
    if thresholds.crap.is_none() {
        thresholds.crap = config.thresholds.crap;
    }
    if thresholds.max_nesting.is_none() {
        thresholds.max_nesting = config.thresholds.max_nesting;
    }
}

/// Regression limits from the request, falling back to `[regressions]`.
///
/// `true` means "use the configured limits"; an object starts from the
/// configured limits and overrides the fields it names, mirroring how
/// thresholds treat explicit flags.
fn parse_regression_limits(
    params: &serde_json::Value,
    config: Option<&crate::config::Config>,
) -> Result<Option<RegressionLimits>, (i64, String)> {
    let configured = config
        .map(|selected| selected.regressions.clone())
        .unwrap_or_default();
    let Some(value) = params.get("regressions") else {
        return Ok(None);
    };
    if value.is_null() || value == &serde_json::Value::Bool(false) {
        return Ok(None);
    }
    if value == &serde_json::Value::Bool(true) {
        return Ok(Some(configured));
    }
    let object = value
        .as_object()
        .ok_or((-32602, "regressions must be true or an object".to_owned()))?;
    reject_unknown_gate_keys(object, "regressions")?;
    let mut limits = configured;
    for key in ["cognitive", "cyclomatic", "max_nesting"] {
        if let Some(value) = object.get(key) {
            let limit = value.as_u64().ok_or((
                -32602,
                format!("regressions.{key} must be a non-negative integer"),
            ))?;
            let limit = u32::try_from(limit)
                .map_err(|_| (-32602, format!("regressions.{key} must fit in u32")))?;
            match key {
                "cognitive" => limits.cognitive = limit,
                "cyclomatic" => limits.cyclomatic = limit,
                "max_nesting" => limits.max_nesting = limit,
                _ => unreachable!("regression key set and this match must stay in sync"),
            }
        }
    }
    if let Some(value) = object.get("crap") {
        let limit = value.as_f64().ok_or((
            -32602,
            "regressions.crap must be a non-negative number".to_owned(),
        ))?;
        if !limit.is_finite() || limit < 0.0 {
            return Err((
                -32602,
                "regressions.crap must be a non-negative number".to_owned(),
            ));
        }
        limits.crap = limit;
    }
    Ok(Some(limits))
}

fn parse_thresholds(
    params: &serde_json::Value,
    allow_empty: bool,
) -> Result<Thresholds, (i64, String)> {
    let object = match params.get("thresholds") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_object()
                .ok_or((-32602, "thresholds must be an object".to_owned()))?,
        ),
    };
    let mut thresholds = Thresholds {
        cognitive: None,
        cyclomatic: None,
        crap: None,
        max_nesting: None,
    };
    if let Some(object) = object {
        reject_unknown_gate_keys(object, "thresholds")?;
        for key in GATE_KEYS {
            if let Some(value) = object.get(key) {
                if value.is_null() {
                    continue;
                }
                match key {
                    "crap" => {
                        let limit = value.as_f64().ok_or((
                            -32602,
                            "thresholds.crap must be a non-negative number".to_owned(),
                        ))?;
                        if !limit.is_finite() || limit < 0.0 {
                            return Err((
                                -32602,
                                "thresholds.crap must be a non-negative number".to_owned(),
                            ));
                        }
                        thresholds.crap = Some(limit);
                    }
                    _ => {
                        let limit = value.as_u64().ok_or((
                            -32602,
                            format!("thresholds.{key} must be a non-negative integer"),
                        ))?;
                        let limit = u32::try_from(limit)
                            .map_err(|_| (-32602, format!("thresholds.{key} must fit in u32")))?;
                        match key {
                            "cognitive" => thresholds.cognitive = Some(limit),
                            "cyclomatic" => thresholds.cyclomatic = Some(limit),
                            "max_nesting" => thresholds.max_nesting = Some(limit),
                            _ => unreachable!("GATE_KEYS and this match must stay in sync"),
                        }
                    }
                }
            }
        }
    }
    if thresholds.is_empty() && !allow_empty {
        return Err((
            -32602,
            "check requires at least one threshold (e.g. thresholds {cognitive:15, cyclomatic:10}) or regressions with base/baseline".to_owned(),
        ));
    }
    Ok(thresholds)
}

fn tool_explain_metric(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "explain_metric", &["metric"])?;
    let metric = req_str(params, "metric")?;
    let name = metric.to_ascii_lowercase();
    let definition = match name.as_str() {
        "cyclomatic" => {
            "Cyclomatic complexity (default): every function starts at 1. Java: add 1 for each `if`, loop, non-default switch case, ternary expression, `&&`, `||`, and each `->` (lambda/switch arrow, counted in the enclosing function); `catch` adds nothing. JavaScript/TypeScript: add 1 for each `if`, loop, `catch`, non-default switch case, ternary expression, `throw`, `&&`, and `||` (`??` adds nothing). `else`, `finally`, and default cases add nothing. Nested functions are scored independently."
        }
        "cognitive" => {
            "Cognitive complexity (default): add `1 + current nesting` for `if`, loops, `catch`, `switch`, and ternary expressions. An `else if` adds 1 and continues the chain; a final `else` adds 1; a labeled `break`/`continue` adds 1. For logical expressions the first `&&`/`||` adds 1 and each operator change adds 1. Lambdas, arrows, and nested functions are scored independently and do not raise the enclosing score. Direct self-recursion adds 1; mutual cycles are not scored."
        }
        "halstead" => {
            "Halstead metrics (default): leaf operator tokens and control keywords are operators; identifiers, literals, `this`, `super`, `true`, `false`, and `null` are operands. Distinct values are source byte slices within one function. Vocabulary is n1 + n2, length is N1 + N2, volume is length * log2(vocabulary), difficulty is (n1 / 2) * (N2 / n2), effort is difficulty * volume. Zero denominators produce zero, never NaN."
        }
        "maintainability" => {
            "Maintainability index (default): max(0, (171 - 5.2 ln(volume) - 0.23 cyclomatic - 16.2 ln(LOC)) * 100 / 171). Volume and LOC use a floor of 1, the result is capped at 100, and comment weighting is not used. Higher is better."
        }
        "crap" => {
            "CRAP (default): c^2 * (1 - p)^3 + c, where c is cyclomatic complexity and p is normalized function coverage (0.0..1.0). CRAP is null when no known line or branch record overlaps the function range, never a fake 0%. Lower is better."
        }
        _ => {
            return Err((
                -32602,
                format!(
                    "unknown metric '{metric}': expected cyclomatic|cognitive|halstead|maintainability|crap"
                ),
            ));
        }
    };
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("explain_metric"));
    fields.insert("metric".to_owned(), serde_json::json!(name));
    fields.insert("spec".to_owned(), serde_json::json!(METRIC_PROFILE));
    fields.insert("definition".to_owned(), serde_json::Value::from(definition));
    envelope(fields)
}

fn tool_repo_summary(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "repo_summary", &["path", "top", "coverage"])?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let top = match params.get("top") {
        None | Some(serde_json::Value::Null) => 5,
        Some(value) => {
            let count = value
                .as_u64()
                .ok_or((-32602, "top must be a positive integer".to_owned()))?;
            let count =
                usize::try_from(count).map_err(|_| (-32602, "top must fit in usize".to_owned()))?;
            if count == 0 {
                return Err((-32602, "top must be at least 1".to_owned()));
            }
            if count > 50 {
                return Err((-32602, "top must be at most 50".to_owned()));
            }
            count
        }
    };
    let coverage = opt_str(params, "coverage")?
        .map(load_coverage_file)
        .transpose()
        .map_err(|message| (-32602, message))?;
    let report = {
        let config = load_config(path)?;
        let excludes = config_excludes(config.as_ref());
        crate::analyze_path_with_excludes(Path::new(path), coverage.as_ref(), &excludes)
            .map_err(|error| (-32602, error.to_string()))?
    };
    let files = report.files.len();
    let mut functions = 0usize;
    let mut parse_errors = 0usize;
    let mut rows = Vec::new();
    for file in &report.files {
        parse_errors += file.parse_errors.len();
        for function in &file.functions {
            functions += 1;
            rows.push(compact_function(&file.path, function));
        }
    }
    let top_rows = |key: SortKey| {
        let mut selected = rows.clone();
        sort_analyze_rows(&mut selected, key);
        selected.truncate(top);
        selected
    };
    let mut top_fields = serde_json::Map::new();
    if coverage.is_some() {
        top_fields.insert(
            "crap".to_owned(),
            serde_json::json!(top_rows(SortKey::Crap)),
        );
    }
    top_fields.insert(
        "cognitive".to_owned(),
        serde_json::json!(top_rows(SortKey::Cognitive)),
    );
    top_fields.insert(
        "cyclomatic".to_owned(),
        serde_json::json!(top_rows(SortKey::Cyclomatic)),
    );
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("repo_summary"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert(
        "totals".to_owned(),
        serde_json::json!({
            "files": files,
            "functions": functions,
            "parse_errors": parse_errors,
        }),
    );
    fields.insert("top".to_owned(), serde_json::Value::Object(top_fields));
    fields.insert(
        "truncated".to_owned(),
        serde_json::Value::from(functions > top),
    );
    envelope(fields)
}

fn tool_test_targets(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "test_targets", &["path", "coverage", "top"])?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let coverage_path = opt_str(params, "coverage")?.ok_or((
        -32602,
        "test_targets requires a coverage file path".to_owned(),
    ))?;
    let top = parse_top_param(params, MAX_ENTRIES)?;
    let coverage = load_coverage_file(coverage_path).map_err(|message| (-32602, message))?;
    let config = load_config(path)?;
    let excludes = config_excludes(config.as_ref());
    let report = crate::analyze_path_with_excludes(Path::new(path), Some(&coverage), &excludes)
        .map_err(|error| (-32602, error.to_string()))?;
    let ranked = crate::test_targets::test_targets(&report, &coverage);
    let total = ranked.len();
    let limit = top.min(total);
    let truncated = total > limit;
    let mut rows = Vec::with_capacity(limit);
    for target in &ranked[..limit] {
        rows.push(serde_json::json!({
            "path": target.path,
            "function": target.function,
            "line": target.line,
            "crap": round1_opt(target.crap),
            "coverage": round1_opt(target.coverage),
            "uncovered": target.uncovered,
            "unknown": target.unknown,
        }));
    }
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("test_targets"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("targets".to_owned(), serde_json::Value::Array(rows));
    fields.insert("truncated".to_owned(), serde_json::Value::from(truncated));
    fields.insert("total".to_owned(), serde_json::Value::from(total as u64));
    envelope(fields)
}

fn tool_dependencies(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "dependencies", &["path"])?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let config = load_config(path)?;
    let excludes = config_excludes(config.as_ref());
    let report = crate::graph::analyze_dependencies(Path::new(path), &excludes)
        .map_err(|error| (-32602, error.to_string()))?;
    if report.files.is_empty() {
        return Err((-32602, format!("no supported files found under {path}")));
    }
    let agent = crate::agent::dependencies_agent_json(&report);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("dependencies"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    merge_agent_fields(
        &mut fields,
        &agent,
        &[
            "summary",
            "files",
            "edges",
            "cycles",
            "unresolved",
            "truncated",
        ],
    );
    envelope(fields)
}

fn tool_impact(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "impact", &["target", "path", "top"])?;
    let target = req_str(params, "target")?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let top = parse_top_param(params, 20)?;
    let root = Path::new(path);
    if !root.is_dir() {
        return Err((-32602, format!("impact path '{path}' is not a directory")));
    }
    let key =
        crate::resolve_scoped_target(root, target).map_err(|error| (-32602, error.to_string()))?;
    let config = load_config(path)?;
    let excludes = config_excludes(config.as_ref());
    let graph = crate::graph::analyze_dependencies(root, &excludes)
        .map_err(|error| (-32602, error.to_string()))?;
    let Some(report) = crate::impact::analyze_impact(&graph, &key, top) else {
        return Err((
            -32602,
            format!("impact target '{target}' was not found in the dependency graph under {path}"),
        ));
    };
    let agent = crate::agent::impact_agent_json(&report);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("impact"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    merge_agent_fields(
        &mut fields,
        &agent,
        &[
            "model",
            "target",
            "files_analyzed",
            "fan_in",
            "fan_out",
            "direct_dependents",
            "blast_radius",
            "blast_radius_percent",
            "dependents",
            "cycles",
            "truncated",
        ],
    );
    envelope(fields)
}

fn tool_coupling(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "coupling",
        &["target", "path", "top", "min_cochanges"],
    )?;
    let target = req_str(params, "target")?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let top = parse_top_param(params, 20)?;
    let min_cochanges = match params.get("min_cochanges") {
        None | Some(serde_json::Value::Null) => 2_u64,
        Some(value) => {
            let count = value.as_u64().ok_or((
                -32602,
                "min_cochanges must be a positive integer".to_owned(),
            ))?;
            if count == 0 {
                return Err((-32602, "min_cochanges must be at least 1".to_owned()));
            }
            count
        }
    };
    let root = Path::new(path);
    if !root.is_dir() {
        return Err((-32602, format!("coupling path '{path}' is not a directory")));
    }
    let key =
        crate::resolve_scoped_target(root, target).map_err(|error| (-32602, error.to_string()))?;
    let options = crate::coupling::CouplingOptions {
        min_co_changes: min_cochanges,
        limit: top,
    };
    let report = crate::coupling::analyze_coupling(root, &key, &options)
        .map_err(|error| (-32602, error.to_string()))?;
    let agent = crate::agent::coupling_agent_json(&report);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("coupling"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    merge_agent_fields(
        &mut fields,
        &agent,
        &[
            "target",
            "git_available",
            "target_commits",
            "related",
            "truncated",
        ],
    );
    envelope(fields)
}

fn tool_hotspots(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "hotspots", &["path", "top", "since", "coverage"])?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let top = parse_top_param(params, 10)?;
    let window = parse_since(params, crate::history::HistoryWindow::Days90)?;
    let (analysis, _) = analyze_optional_coverage(params, path)?;
    let history = crate::history::analyze_history(&config_dir(path))
        .map_err(|error| (-32602, error.to_string()))?;
    let report = crate::hotspots::build(&analysis, &history, window, top);
    let agent = crate::agent::hotspots_agent_json(&report);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("hotspots"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    merge_agent_fields(
        &mut fields,
        &agent,
        &[
            "model",
            "window",
            "git_available",
            "summary",
            "hotspots",
            "truncated",
        ],
    );
    envelope(fields)
}

fn tool_duplication(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "duplication", &["path", "base"])?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let base = opt_str(params, "base")?;
    let outcome = crate::analytics::analyze_duplication(
        Path::new(path),
        crate::source_snapshot::SnapshotTarget::Worktree,
        base,
    )
    .map_err(|error| (-32602, error.to_string()))?;
    let (mode, mut fields) = match &outcome {
        crate::analytics::DuplicationOutcome::Single(report) => ("single", report_object(report)?),
        crate::analytics::DuplicationOutcome::Drift(report) => ("drift", report_object(report)?),
    };
    fields.insert("tool".to_owned(), serde_json::json!("duplication"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("base".to_owned(), serde_json::json!(base));
    fields.insert("mode".to_owned(), serde_json::json!(mode));
    envelope(fields)
}

fn tool_policy(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "policy", &["path", "base"])?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let base = opt_str(params, "base")?;
    let report = crate::analytics::analyze_policy(
        Path::new(path),
        crate::source_snapshot::SnapshotTarget::Worktree,
        base,
    )
    .map_err(|error| (-32602, error.to_string()))?;
    let mut fields = report_object(&report)?;
    fields.insert("tool".to_owned(), serde_json::json!("policy"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("base".to_owned(), serde_json::json!(base));
    envelope(fields)
}

fn tool_risk(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(params, "risk", &["path", "top", "since", "coverage"])?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let top = parse_top_param(params, 10)?;
    let window = parse_since(params, crate::history::HistoryWindow::Days90)?;
    let (analysis, excludes) = analyze_optional_coverage(params, path)?;
    let mut report = crate::analytics::analyze_risk(&analysis, Path::new(path), &excludes, window)
        .map_err(|error| (-32602, error.to_string()))?;
    report.risks.truncate(top);
    let agent = crate::agent::risk_agent_json(&report);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("risk"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    merge_agent_fields(
        &mut fields,
        &agent,
        &["model", "window", "git_available", "summary", "risks"],
    );
    envelope(fields)
}

fn analyze_optional_coverage(
    params: &serde_json::Value,
    path: &str,
) -> Result<(crate::core::AnalysisReport, Vec<String>), (i64, String)> {
    let coverage = opt_str(params, "coverage")?
        .map(load_coverage_file)
        .transpose()
        .map_err(|message| (-32602, message))?;
    let config = load_config(path)?;
    let excludes = config_excludes(config.as_ref());
    let analysis = crate::analyze_path_with_excludes(Path::new(path), coverage.as_ref(), &excludes)
        .map_err(|error| (-32602, error.to_string()))?;
    if analysis.files.is_empty() {
        return Err((-32602, format!("no supported files found under {path}")));
    }
    Ok((analysis, excludes))
}

fn tool_debt(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "debt",
        &["path", "base", "target", "renames", "since"],
    )?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let base = opt_str(params, "base")?.unwrap_or("HEAD~1");
    let target = match opt_str(params, "target")? {
        None | Some("worktree") => crate::source_snapshot::SnapshotTarget::Worktree,
        Some("index") => crate::source_snapshot::SnapshotTarget::Index,
        Some(revision) => crate::source_snapshot::SnapshotTarget::Revision(revision.to_owned()),
    };
    let renames = opt_flag(params, "renames", "debt")?;
    let window = parse_since(params, crate::history::HistoryWindow::Days90)?;
    let report = crate::analytics::analyze_debt(&crate::analytics::DebtRequest {
        path: std::path::PathBuf::from(path),
        base: base.to_owned(),
        target,
        detect_renames: renames,
        window,
        fail_on_regression: false,
    })
    .map_err(|error| (-32602, error.to_string()))?;
    let agent = crate::agent::debt_agent_json(&report);
    crate::telemetry::record_debt("mcp", &report.summary);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("debt"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("base".to_owned(), serde_json::json!(base));
    merge_agent_fields(
        &mut fields,
        &agent,
        &[
            "model",
            "target_commit",
            "summary",
            "findings",
            "risk_changes",
            "truncated",
        ],
    );
    envelope(fields)
}

fn tool_project(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "project",
        &[
            "path",
            "target",
            "since",
            "coverage",
            "ownership",
            "pit",
            "stryker",
            "test_map",
            "snapshots",
        ],
    )?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let target = match opt_str(params, "target")? {
        None => crate::source_snapshot::SnapshotTarget::Worktree,
        Some(revision) => crate::source_snapshot::SnapshotTarget::Revision(revision.to_owned()),
    };
    let window = parse_since(params, crate::history::HistoryWindow::Days90)?;
    let coverage = opt_str(params, "coverage")?
        .map(load_coverage_file)
        .transpose()
        .map_err(|message| (-32602, message))?;
    let ownership_mode = match opt_str(params, "ownership")? {
        None | Some("aggregate") => crate::ownership::OwnershipMode::AggregateOnly,
        Some("include_authors") => crate::ownership::OwnershipMode::IncludeAuthors,
        Some("anonymize_authors") => crate::ownership::OwnershipMode::AnonymizeAuthors,
        Some(other) => {
            return Err((
                -32602,
                format!(
                    "unknown ownership '{other}': expected 'aggregate', 'include_authors', or 'anonymize_authors'"
                ),
            ));
        }
    };
    let mut mutation_inputs = Vec::new();
    for pit in artifact_paths(params, "project", "pit", false)? {
        mutation_inputs.push(crate::mutation::MutationInput::Pit(pit));
    }
    for stryker in artifact_paths(params, "project", "stryker", false)? {
        mutation_inputs.push(crate::mutation::MutationInput::Stryker(stryker));
    }
    let test_maps = artifact_paths(params, "project", "test_map", false)?;
    let snapshots_path = opt_str(params, "snapshots")?
        .map(|file| artifact_path("project", "snapshots", file))
        .transpose()?;
    let report = crate::analytics::build(&crate::analytics::ProjectRequest {
        path: std::path::PathBuf::from(path),
        target,
        window,
        mutation_inputs,
        test_maps,
        ownership_mode,
        snapshots_path,
        coverage,
    })
    .map_err(|error| (-32602, error.to_string()))?;
    let agent = crate::agent::project_agent_json(&report);
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("project"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    merge_agent_fields(
        &mut fields,
        &agent,
        &[
            "generated_from",
            "git_available",
            "head_commit",
            "summary",
            "risk",
            "truncated",
        ],
    );
    envelope(fields)
}

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

/// Load one coverage file, auto-detecting the format by extension:
/// `.xml` is JaCoCo XML, anything else (including `.info`) is LCOV.
fn load_coverage_file(path: &str) -> Result<crate::coverage::CoverageMap, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read coverage '{path}': {error}"))?;
    if Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
    {
        crate::coverage::CoverageMap::from_jacoco_xml(&text)
    } else {
        crate::coverage::CoverageMap::from_lcov(&text)
    }
    .map_err(|error| format!("cannot parse coverage '{path}': {error}"))
}

// ---------------------------------------------------------------------------
// Protocol helpers
// ---------------------------------------------------------------------------

/// Builds the initialize result, echoing the client's requested protocol
/// version per the spec's negotiation rules. Everything this server
/// implements (initialize, tools/list, tools/call, ping, notifications) is
/// version-stable, so echoing is honest; `2024-11-05` remains the fallback
/// for clients that send no version.
fn initialize_result(params: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": params
            .get("protocolVersion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("2024-11-05"),
        "capabilities": { "tools": {} },
        "serverInfo": {
            "name": "leadline",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": SERVER_INSTRUCTIONS,
    })
}

/// Compare checked-in PostgreSQL plan directories without touching a database.
///
/// `current` and `baseline` stay root-relative; absolute paths are rejected.
/// Unknown arguments, non-finite or negative limits, and `top` above the
/// global entry cap are invalid-request errors.
/// Reject unknown object fields so typos fail loudly instead of silently.
fn reject_unknown(
    params: &serde_json::Value,
    tool: &str,
    known: &[&str],
) -> Result<(), (i64, String)> {
    if let Some(object) = params.as_object() {
        for key in object.keys() {
            if !known.contains(&key.as_str()) {
                return Err((-32602, format!("unknown {tool} argument '{key}'")));
            }
        }
    }
    Ok(())
}

/// Gate names `thresholds` and `regressions` share.
const GATE_KEYS: [&str; 4] = ["cognitive", "cyclomatic", "crap", "max_nesting"];

/// Reject unknown gate names so a typo cannot silently weaken a gate.
fn reject_unknown_gate_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    section: &str,
) -> Result<(), (i64, String)> {
    for key in object.keys() {
        if !GATE_KEYS.contains(&key.as_str()) {
            return Err((
                -32602,
                format!(
                    "unknown {section} key '{key}': expected one of {}",
                    GATE_KEYS.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

/// Shared `top` row cap: 1..=MAX_ENTRIES, defaulting when absent or null.
fn parse_top_param(params: &serde_json::Value, default: usize) -> Result<usize, (i64, String)> {
    match params.get("top") {
        None | Some(serde_json::Value::Null) => Ok(default),
        Some(value) => {
            let count = value
                .as_u64()
                .ok_or((-32602, "top must be a positive integer".to_owned()))?;
            let count =
                usize::try_from(count).map_err(|_| (-32602, "top must fit in usize".to_owned()))?;
            if count == 0 {
                return Err((-32602, "top must be at least 1".to_owned()));
            }
            if count > MAX_ENTRIES {
                return Err((-32602, format!("top must be at most {MAX_ENTRIES}")));
            }
            Ok(count)
        }
    }
}

/// One `since` window: `30d`, `90d`, or `365d`.
fn parse_since(
    params: &serde_json::Value,
    default: crate::history::HistoryWindow,
) -> Result<crate::history::HistoryWindow, (i64, String)> {
    match opt_str(params, "since")? {
        None => Ok(default),
        Some(raw) => crate::history::HistoryWindow::parse(raw).ok_or((
            -32602,
            format!("unknown since '{raw}': expected '30d', '90d', or '365d'"),
        )),
    }
}

/// `leadline.toml` from the analysis root of a tool's `path` argument.
///
/// MCP tools must match the CLI: `[analysis]`, `[sql]`, and
/// `[vulnerabilities]` all apply. A broken config is an invalid-params error
/// so hosts see the same typo the CLI rejects.
fn load_config(path: &str) -> Result<Option<crate::config::Config>, (i64, String)> {
    crate::config::load_from(&config_dir(path)).map_err(|error| (-32602, error.to_string()))
}

/// Analysis-root directory a tool's `path` points at: the path itself when it
/// names a directory, its parent when it names a file, and the working
/// directory when neither exists.
fn config_dir(path: &str) -> PathBuf {
    let path = Path::new(path);
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    };
    if dir.is_dir() {
        dir
    } else {
        PathBuf::from(".")
    }
}

/// `[analysis].exclude` for one tool path; empty without a config file.
fn config_excludes(config: Option<&crate::config::Config>) -> Vec<String> {
    config
        .map(|selected| selected.analysis_excludes.clone())
        .unwrap_or_default()
}

/// Index directory a read-only tool should warm from: the explicit `index`
/// argument when given (used verbatim), else `[index].path` from
/// configuration joined onto the analysis root like the CLI, else nothing.
/// These callers only ever read it.
fn index_directory(
    params: &serde_json::Value,
    config: Option<&crate::config::Config>,
    path: &str,
) -> Result<Option<PathBuf>, (i64, String)> {
    if let Some(argument) = opt_str(params, "index")? {
        return Ok(Some(PathBuf::from(argument)));
    }
    Ok(config
        .and_then(|selected| selected.index.as_ref())
        .map(|configured| config_dir(path).join(&configured.path)))
}

struct CachedIndex {
    modified: Option<SystemTime>,
    len: u64,
    index: Arc<crate::index::AnalysisIndex>,
}

static INDEX_CACHE: OnceLock<Mutex<HashMap<std::path::PathBuf, CachedIndex>>> = OnceLock::new();

/// Opens the index at `dir` once per file identity (path + mtime + length) and
/// reuses the parsed value afterwards. A missing file caches the empty index.
fn cached_index(dir: &std::path::Path) -> Arc<crate::index::AnalysisIndex> {
    let path = dir.join(crate::index::INDEX_FILE_NAME);
    let metadata = std::fs::metadata(&path).ok();
    let cache = INDEX_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(cached) = cache.lock().ok().and_then(|cache| {
        cache
            .get(&path)
            .map(|cached| (cached.modified, cached.len, Arc::clone(&cached.index)))
    }) {
        let (modified, len, index) = cached;
        let fresh = metadata.as_ref().map_or(modified.is_none(), |metadata| {
            metadata.modified().ok() == modified && metadata.len() == len
        });
        if fresh {
            return index;
        }
    }
    let index = Arc::new(crate::index::AnalysisIndex::open(dir));
    let (modified, len) = metadata.map_or((None, 0), |metadata| {
        (metadata.modified().ok(), metadata.len())
    });
    if let Ok(mut cache) = cache.lock() {
        cache.insert(
            path,
            CachedIndex {
                modified,
                len,
                index: Arc::clone(&index),
            },
        );
    }
    index
}

/// Report for `analyze` and `check`: reuse a stored index when one is
/// resolved and no coverage is applied, else measure the path cold.
///
/// Returns the reuse counts only when the warm path produced the report; a
/// missing or unusable index silently falls back to the cold analysis.
fn analyze_read_only(
    params: &serde_json::Value,
    config: Option<&crate::config::Config>,
    path: &str,
    coverage: Option<&crate::coverage::CoverageMap>,
    excludes: &[String],
) -> Result<(crate::core::AnalysisReport, Option<crate::index::Reuse>), (i64, String)> {
    let index_dir = index_directory(params, config, path)?;
    if coverage.is_none()
        && let Some(dir) = index_dir
    {
        let previous = cached_index(&dir);
        if let Ok((report, reuse)) = crate::index::warm_report_with(
            &previous,
            Path::new(path),
            &crate::index::scope_label(Path::new(path)),
            &crate::config::fingerprint(&config_dir(path)),
            excludes,
        ) {
            return Ok((report, Some(reuse)));
        }
    }
    let report = crate::analyze_path_with_excludes(Path::new(path), coverage, excludes)
        .map_err(|error| (-32602, error.to_string()))?;
    Ok((report, None))
}

/// Envelope `index` block for a report that reused a stored index.
fn index_counts(reuse: Option<crate::index::Reuse>) -> Option<serde_json::Value> {
    reuse.map(|reuse| {
        serde_json::json!({
            "reused": reuse.reused,
            "analyzed": reuse.analyzed,
        })
    })
}

/// Optional boolean flag, defaulting to false when absent or null.
fn opt_flag(params: &serde_json::Value, key: &str, tool: &str) -> Result<bool, (i64, String)> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(false),
        Some(value) => value
            .as_bool()
            .ok_or((-32602, format!("{tool} '{key}' must be a boolean"))),
    }
}

/// Required-or-optional array of path strings (max 32); shape validation
/// only, callers choose lexical or analysis-root resolution.
fn path_arguments(
    params: &serde_json::Value,
    tool: &str,
    key: &str,
    required: bool,
) -> Result<Vec<String>, (i64, String)> {
    let Some(value) = params.get(key) else {
        if required {
            return Err((-32602, format!("{tool} requires '{key}'")));
        }
        return Ok(Vec::new());
    };
    if value.is_null() && !required {
        return Ok(Vec::new());
    }
    let entries = value
        .as_array()
        .ok_or((-32602, format!("{tool} '{key}' must be an array of paths")))?;
    if entries.len() > 32 {
        return Err((-32602, format!("{tool} '{key}' accepts at most 32 entries")));
    }
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or((-32602, format!("{tool} '{key}' entries must be strings")))
        })
        .collect()
}

/// Lexically normalize one root-relative path: native separators are accepted;
/// absolute paths, parent escapes, and control characters are rejected.
fn relative_path(tool: &str, key: &str, path: &str) -> Result<String, (i64, String)> {
    let validate = |path| {
        crate::external::strict_relative_path(path).map_err(|error| {
            (
                -32602,
                format!("{tool} '{key}' must be root-relative: {error}"),
            )
        })
    };
    let native = path.replace('\\', "/");
    let normalized = validate(&native)?;
    validate(&normalized)
}

/// Required-or-optional array of artifact paths that must stay inside the
/// working directory.
fn artifact_paths(
    params: &serde_json::Value,
    tool: &str,
    key: &str,
    required: bool,
) -> Result<Vec<PathBuf>, (i64, String)> {
    path_arguments(params, tool, key, required)?
        .into_iter()
        .map(|raw| {
            let normalized = relative_path(tool, key, &raw)?;
            contained_artifact_path(tool, key, &normalized)
        })
        .collect()
}

/// Resolve one lexically normalized artifact path, rejecting paths that
/// resolve outside the working directory.
///
/// The full path is resolved when it exists; otherwise its parent directory
/// is, so a missing artifact cannot hide behind a symlinked directory. The
/// later read resolves again, so only a local swap between the two resolves
/// can still race.
fn contained_artifact_path(
    tool: &str,
    key: &str,
    normalized: &str,
) -> Result<PathBuf, (i64, String)> {
    let outside = || {
        (
            -32602,
            format!("{tool} '{key}' must stay inside the working directory"),
        )
    };
    let root = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .map_err(|_| outside())?;
    let candidate = Path::new(normalized);
    let resolved = std::fs::canonicalize(candidate).ok().or_else(|| {
        let parent = candidate
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())?;
        std::fs::canonicalize(parent).ok()
    });
    if resolved.is_some_and(|resolved| !resolved.starts_with(&root)) {
        return Err(outside());
    }
    Ok(PathBuf::from(normalized))
}

/// Validate and resolve one root-relative artifact path.
fn artifact_path(tool: &str, key: &str, path: &str) -> Result<PathBuf, (i64, String)> {
    let normalized = relative_path(tool, key, path)?;
    contained_artifact_path(tool, key, &normalized)
}

/// Triage scanner SARIF with code context, without running scanners.
///
/// Artifact arguments stay root-relative; absolute paths, unknown fields,
/// non-array inputs, bad severities, and over-limit `top` are rejected.
fn tool_security_findings(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "security_findings",
        &[
            "path",
            "sarif",
            "baseline_sarif",
            "base",
            "staged",
            "target",
            "minimum_severity",
            "new_only",
            "changed_only",
            "top",
        ],
    )?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let sarif = artifact_paths(params, "security_findings", "sarif", true)?;
    if sarif.is_empty() {
        return Err((
            -32602,
            "security_findings requires at least one 'sarif' file".to_owned(),
        ));
    }
    let baseline_sarif = artifact_paths(params, "security_findings", "baseline_sarif", false)?;
    let base = opt_str(params, "base")?;
    let staged = opt_flag(params, "staged", "security_findings")?;
    let target = opt_str(params, "target")?;
    match (base.is_some(), staged, target.is_some()) {
        (true, true, _) => {
            return Err((-32602, "base and staged are exclusive".to_owned()));
        }
        (_, true, true) => {
            return Err((-32602, "staged and target are exclusive".to_owned()));
        }
        (true, _, true) => {
            return Err((-32602, "base and target are exclusive".to_owned()));
        }
        _ => {}
    }
    let new_only = opt_flag(params, "new_only", "security_findings")?;
    let changed_only = opt_flag(params, "changed_only", "security_findings")?;
    let comparison = match (base, staged, target) {
        (Some(revision), false, None) => {
            Some(crate::security::ChangeComparison::Base(revision.to_owned()))
        }
        (None, true, None) => Some(crate::security::ChangeComparison::Staged),
        (None, false, Some(revision)) => Some(crate::security::ChangeComparison::Target(
            revision.to_owned(),
        )),
        (None, false, None) => None,
        _ => unreachable!("exclusive comparison flags are rejected above"),
    };
    if changed_only && comparison.is_none() {
        return Err((
            -32602,
            "changed_only requires base, staged, or target".to_owned(),
        ));
    }
    let gate = match opt_str(params, "minimum_severity")? {
        None => None,
        Some(raw) => Some(crate::security::SecurityGate {
            minimum: crate::security::parse_gate_severity(raw).ok_or((
                -32602,
                format!("unknown minimum_severity '{raw}': expected 'low', 'medium', 'high', or 'critical'"),
            ))?,
            new_only,
            changed_only,
        }),
    };
    let top = parse_top_param(params, crate::security::AGENT_DEFAULT_TOP)?;
    let outcome = crate::security::assemble(&crate::security::SecurityRequest {
        path: PathBuf::from(path),
        sarif,
        baseline_sarif,
        comparison,
        gate,
    })
    .map_err(|error| (-32602, error.message().to_owned()))?;
    let agent = crate::security::agent_json(&outcome.report, top);
    let (violations, violations_truncated) = cap_violations(&outcome.violations, top)?;
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("security_findings"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert(
        "findings".to_owned(),
        agent.get("findings").cloned().unwrap_or_default(),
    );
    fields.insert("violations".to_owned(), violations);
    fields.insert(
        "truncated".to_owned(),
        serde_json::Value::from(
            agent
                .get("truncated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                || violations_truncated,
        ),
    );
    envelope(fields)
}

/// Prioritize vulnerable dependencies with changed-import evidence.
///
/// Scanner artifacts stay root-relative (max 32 each); at least one of
/// `osv`/`trivy` is required. Unknown fields, non-array inputs, absolute
/// paths, bad severities, and over-limit `top` are rejected.
fn tool_vulnerabilities(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "vulnerabilities",
        &[
            "path",
            "osv",
            "trivy",
            "baseline_osv",
            "baseline_trivy",
            "base",
            "staged",
            "target",
            "minimum_severity",
            "top",
        ],
    )?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let osv = artifact_paths(params, "vulnerabilities", "osv", false)?;
    let trivy = artifact_paths(params, "vulnerabilities", "trivy", false)?;
    if osv.is_empty() && trivy.is_empty() {
        return Err((
            -32602,
            "vulnerabilities requires at least one of 'osv' or 'trivy'".to_owned(),
        ));
    }
    let baseline_osv = artifact_paths(params, "vulnerabilities", "baseline_osv", false)?;
    let baseline_trivy = artifact_paths(params, "vulnerabilities", "baseline_trivy", false)?;
    let base = opt_str(params, "base")?;
    let staged = opt_flag(params, "staged", "vulnerabilities")?;
    let target = opt_str(params, "target")?;
    match (base.is_some(), staged, target.is_some()) {
        (true, true, _) => {
            return Err((-32602, "base and staged are exclusive".to_owned()));
        }
        (_, true, true) => {
            return Err((-32602, "staged and target are exclusive".to_owned()));
        }
        (true, _, true) => {
            return Err((-32602, "base and target are exclusive".to_owned()));
        }
        _ => {}
    }
    let comparison = match (base, staged, target) {
        (Some(revision), false, None) => {
            Some(crate::security::ChangeComparison::Base(revision.to_owned()))
        }
        (None, true, None) => Some(crate::security::ChangeComparison::Staged),
        (None, false, Some(revision)) => Some(crate::security::ChangeComparison::Target(
            revision.to_owned(),
        )),
        (None, false, None) => None,
        _ => unreachable!("exclusive comparison flags are rejected above"),
    };
    let config = load_config(path)?;
    let gate = match opt_str(params, "minimum_severity")? {
        Some(raw) => Some(crate::security::parse_gate_severity(raw).ok_or((
            -32602,
            format!(
                "unknown minimum_severity '{raw}': expected 'low', 'medium', 'high', or 'critical'"
            ),
        ))?),
        None => config
            .as_ref()
            .and_then(|selected| selected.vulnerabilities.minimum_severity),
    };
    let top = parse_top_param(params, crate::security::AGENT_DEFAULT_TOP)?;
    let outcome = crate::vulnerabilities::assemble(&crate::vulnerabilities::VulnerabilityRequest {
        path: PathBuf::from(path),
        osv,
        trivy,
        baseline_osv,
        baseline_trivy,
        comparison,
        gate,
    })
    .map_err(|error| (-32602, error.to_string()))?;
    let agent = crate::vulnerabilities::agent_json(&outcome.report, top);
    let (violations, violations_truncated) = cap_violations(&outcome.violations, top)?;
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("vulnerabilities"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert(
        "findings".to_owned(),
        agent.get("findings").cloned().unwrap_or_default(),
    );
    fields.insert("violations".to_owned(), violations);
    fields.insert(
        "truncated".to_owned(),
        serde_json::Value::from(
            agent
                .get("truncated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                || violations_truncated,
        ),
    );
    fields.insert(
        "reachability_model".to_owned(),
        serde_json::json!("changed-direct-imports"),
    );
    envelope(fields)
}

/// Flag static PostgreSQL query risks without executing SQL.
///
/// Unknown fields, bad limits, absolute migration roots, bad severities,
/// and over-limit `top` are rejected.
fn tool_sql_risks(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "sql_risks",
        &[
            "path",
            "large_offset",
            "migration_roots",
            "minimum_severity",
            "top",
        ],
    )?;
    let path = opt_str(params, "path")?.unwrap_or(".");
    let config = load_config(path)?;
    let defaults = config
        .as_ref()
        .map(|selected| selected.sql.clone())
        .unwrap_or_default();
    let large_offset = match params.get("large_offset") {
        None | Some(serde_json::Value::Null) => defaults.large_offset,
        Some(value) => {
            let threshold = value
                .as_u64()
                .ok_or((-32602, "large_offset must be a positive integer".to_owned()))?;
            if threshold == 0 {
                return Err((-32602, "large_offset must be at least 1".to_owned()));
            }
            threshold
        }
    };
    let migration_roots = match params.get("migration_roots") {
        None | Some(serde_json::Value::Null) => defaults.migration_roots,
        // Migration roots are analysis-root-relative, so working-directory
        // containment must not apply; the CLI normalizer owns lexical rules
        // (including its rejection of interior `..`), keeping exact parity.
        Some(_) => path_arguments(params, "sql_risks", "migration_roots", false)?
            .into_iter()
            .map(|text| {
                crate::config::normalize_migration_root(&text)
                    .map_err(|error| (-32602, error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let gate = match opt_str(params, "minimum_severity")? {
        None => None,
        Some(raw) => Some(crate::security::parse_gate_severity(raw).ok_or((
            -32602,
            format!(
                "unknown minimum_severity '{raw}': expected 'low', 'medium', 'high', or 'critical'"
            ),
        ))?),
    };
    let top = parse_top_param(params, crate::security::AGENT_DEFAULT_TOP)?;
    let excludes = config_excludes(config.as_ref());
    let report = crate::sql::analyze_sql_path(
        Path::new(path),
        &crate::config::SqlConfig {
            large_offset,
            migration_roots,
        },
        &excludes,
    )
    .map_err(|error| (-32602, error.to_string()))?;
    let outcome = crate::sql::outcome(report, gate);
    let agent = crate::sql::agent_json(&outcome.report, top);
    let (violations, violations_truncated) = cap_violations(&outcome.violations, top)?;
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("sql_risks"));
    fields.insert("path".to_owned(), serde_json::json!(path));
    fields.insert("dialect".to_owned(), serde_json::json!("postgresql"));
    fields.insert(
        "schema_evidence_available".to_owned(),
        serde_json::json!(outcome.report.schema_evidence_available),
    );
    fields.insert(
        "findings".to_owned(),
        agent.get("findings").cloned().unwrap_or_default(),
    );
    fields.insert("violations".to_owned(), violations);
    fields.insert(
        "truncated".to_owned(),
        serde_json::Value::from(
            agent
                .get("truncated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                || violations_truncated,
        ),
    );
    envelope(fields)
}

fn tool_sql_plan(params: &serde_json::Value) -> Result<serde_json::Value, (i64, String)> {
    reject_unknown(
        params,
        "sql_plan",
        &[
            "current",
            "baseline",
            "max_cost_increase_percent",
            "max_plan_rows_ratio",
            "max_estimate_error_ratio",
            "top",
        ],
    )?;
    let current =
        opt_str(params, "current")?.ok_or((-32602, "sql_plan requires 'current'".to_owned()))?;
    let baseline =
        opt_str(params, "baseline")?.ok_or((-32602, "sql_plan requires 'baseline'".to_owned()))?;
    let current_path = artifact_path("sql_plan", "current", current)?;
    let baseline_path = artifact_path("sql_plan", "baseline", baseline)?;
    let limits = crate::pg_plan::PlanLimits {
        max_cost_increase_percent: opt_plan_limit(params, "max_cost_increase_percent")?,
        max_plan_rows_ratio: opt_plan_limit(params, "max_plan_rows_ratio")?,
        max_estimate_error_ratio: opt_plan_limit(params, "max_estimate_error_ratio")?,
    };
    let top = parse_top_param(params, crate::security::AGENT_DEFAULT_TOP)?;
    let report = crate::pg_plan::compare_plan_directories(&current_path, &baseline_path, &limits)
        .map_err(|error| (-32602, error.to_string()))?;
    let agent = crate::pg_plan::agent_json(&report, top);
    let (violations, violations_truncated) = cap_violations(&report.violations, top)?;
    let mut fields = serde_json::Map::new();
    fields.insert("tool".to_owned(), serde_json::json!("sql_plan"));
    fields.insert("current".to_owned(), serde_json::json!(current));
    fields.insert("baseline".to_owned(), serde_json::json!(baseline));
    fields.insert(
        "queries".to_owned(),
        agent.get("queries").cloned().unwrap_or_default(),
    );
    fields.insert("violations".to_owned(), violations);
    fields.insert(
        "truncated".to_owned(),
        serde_json::Value::from(
            agent
                .get("truncated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
                || violations_truncated,
        ),
    );
    envelope(fields)
}

/// Optional non-negative finite `sql_plan` gate limit.
fn opt_plan_limit(params: &serde_json::Value, key: &str) -> Result<Option<f64>, (i64, String)> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let limit = value
                .as_f64()
                .ok_or((-32602, format!("sql_plan '{key}' must be a number")))?;
            if !limit.is_finite() || limit < 0.0 {
                return Err((
                    -32602,
                    format!("sql_plan '{key}' must be finite and non-negative"),
                ));
            }
            Ok(Some(limit))
        }
    }
}

fn tools_list_result() -> serde_json::Value {
    serde_json::json!({
        "tools": [
            {
                "name": "analyze",
                "description": "Measure function complexity across a path. Result paths are relative to the `path` argument (rejoin with it before passing one to analyze_function). Use for hotspot lists (sort_by/top) or specific metrics; use analyze_changed after edits.",
                "annotations": { "title": "Analyze complexity", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "coverage": { "type": ["string", "null"], "description": "Coverage file path (.info for LCOV, .xml for JaCoCo)." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Keep at most this many rows." },
                        "sort_by": { "type": "string", "enum": ["crap", "cognitive", "cyclomatic"], "description": "Sort rows by metric descending before capping. sort_by:crap is meaningless without coverage (CRAP null)." },
                        "min_crap": { "type": "number", "description": "Drop functions whose CRAP is below this floor; unknown CRAP is dropped." },
                        "index": { "type": "string", "description": "Directory containing index.json; reuse unchanged analysis. Read-only." },
                    },
                },
            },
            {
                "name": "analyze_changed",
                "description": "Use after editing code to see which functions regressed. Compares against a git base with before/after deltas. Use min_delta to hide line-shift churn (delta 0).",
                "annotations": { "title": "Analyze changed functions", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "base": { "type": "string", "default": "HEAD~1" },
                        "path": { "type": "string", "default": "." },
                        "target": { "type": "string", "default": "worktree", "description": "'worktree', 'index', or a revision to compare against base." },
                        "renames": { "type": "boolean", "default": false, "description": "Detect Git file renames and pair old-path content with new-path content." },
                        "explain": { "type": "boolean", "default": false, "description": "Include multiset-added contribution causes on regression rows." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Keep at most this many changes." },

                        "sort_by": { "type": "string", "enum": ["crap", "cognitive", "cyclomatic"], "description": "Sort changes by current-side metric descending." },
                        "min_crap": { "type": "number", "description": "Drop changes whose current-side CRAP is below this floor." },
                        "min_delta": { "type": "number", "description": "Drop changes whose max absolute delta is below this floor." },
                    },
                },
            },
            {
                "name": "analyze_function",
                "description": "Inspect one named function. Use explain:true to see the exact lines driving its complexity.",
                "annotations": { "title": "Analyze one function", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "function": { "type": "string" },
                        "explain": { "type": "boolean", "default": false, "description": "Include per-decision contribution lines (rule, line, nesting, increments)." },
                    },
                    "required": ["path", "function"],
                },
            },
            {
                "name": "check",
                "description": "Quality gate for thresholds or regressions. Use in CI or before committing; pass coverage so CRAP gates are meaningful. Requires thresholds or regressions with base/baseline. Violation rows carry `reason`: the threshold names that failed, `regression` for a delta gate, or `crap_unavailable` when a CRAP gate found no coverage record for the function (an unavailable CRAP fails closed).",
                "annotations": { "title": "Run quality gate", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "base": { "type": "string", "description": "Git base revision for changed-function gates." },
                        "baseline": { "type": "string", "description": "Baseline snapshot file for regression gates without Git. Exclusive with base; read-only, never written by this server." },
                        "coverage": { "type": "string", "description": "Coverage file path (.info for LCOV, .xml for JaCoCo) applied before CRAP gates." },
                        "index": { "type": "string", "description": "Directory containing index.json; reuse unchanged analysis. Read-only." },
                        "thresholds": {
                            "type": "object",
                            "properties": {
                                "cognitive": { "type": "integer" },
                                "cyclomatic": { "type": "integer" },
                                "crap": { "type": "number" },
                                "max_nesting": { "type": "integer" },
                            },
                        },
                        "regressions": {
                            "description": "true for the configured leadline.toml [regressions] limits (zero without a config), or an object of allowed non-negative deltas overriding those limits.",
                            "type": ["boolean", "object"],
                            "properties": {
                                "cognitive": { "type": "integer", "minimum": 0 },
                                "cyclomatic": { "type": "integer", "minimum": 0 },
                                "crap": { "type": "number", "minimum": 0 },
                                "max_nesting": { "type": "integer", "minimum": 0 },
                            },
                        },
                    },
                },
            },
            {
                "name": "explain_metric",
                "description": "Look up how a metric is defined (default) before interpreting its numbers.",
                "annotations": { "title": "Explain metric", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "metric": { "type": "string", "enum": ["cyclomatic", "cognitive", "halstead", "maintainability", "crap"] },
                    },
                    "required": ["metric"],
                },
            },
            {
                "name": "repo_summary",
                "description": "Use as a first look at unfamiliar code: totals plus the top functions by CRAP, cognitive, and cyclomatic complexity. Result paths are relative to the `path` argument. Pass `coverage` for the CRAP list; without it the `crap` list is omitted because CRAP needs coverage.",
                "annotations": { "title": "Repository summary", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 50, "default": 5, "description": "Functions kept per metric list." },
                        "coverage": { "type": "string", "description": "Coverage file path (.info for LCOV, .xml for JaCoCo) enabling the CRAP list." },
                    },
                },
            },
            {
                "name": "security_findings",
                "description": "Triage scanner SARIF findings with function, risk, and changed-code context. Read-only: never runs scanners; provide pre-generated files, never touches the network.",
                "annotations": { "title": "Triage security findings", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "sarif": { "type": "array", "items": { "type": "string" }, "maxItems": 32, "description": "Root-relative SARIF files." },
                        "baseline_sarif": { "type": "array", "items": { "type": "string" }, "maxItems": 32, "description": "Root-relative baseline SARIF files for new/existing state." },
                        "base": { "type": "string", "description": "Git base revision for changed-state attribution. Exclusive with staged/target." },
                        "staged": { "type": "boolean", "default": false, "description": "Compare against the index. Exclusive with base/target." },
                        "target": { "type": "string", "description": "Revision to compare against base. Exclusive with base/staged." },
                        "minimum_severity": { "type": "string", "enum": ["low", "medium", "high", "critical"], "description": "Gate floor; absent means informational." },
                        "new_only": { "type": "boolean", "default": false, "description": "Gate only new findings." },
                        "changed_only": { "type": "boolean", "default": false, "description": "Gate only changed findings." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Keep at most this many findings." },
                    },
                    "required": ["sarif"],
                },
            },
            {
                "name": "sql_plan",
                "description": "Compare checked-in PostgreSQL EXPLAIN (FORMAT JSON) directories for plan regressions. Read-only: never connects to a database or executes SQL.",
                "annotations": { "title": "Check plan regressions", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "current": { "type": "string", "description": "Root-relative directory of current EXPLAIN JSON files." },
                        "baseline": { "type": "string", "description": "Root-relative directory of baseline EXPLAIN JSON files." },
                        "max_cost_increase_percent": { "type": "number", "minimum": 0, "description": "Fail cost increases past this percent." },
                        "max_plan_rows_ratio": { "type": "number", "minimum": 0, "description": "Fail row growth past this ratio." },
                        "max_estimate_error_ratio": { "type": "number", "minimum": 0, "description": "Fail planner estimate error past this absolute ratio." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Keep at most this many changed queries." },
                    },
                    "required": ["current", "baseline"],
                },
            },
            {
                "name": "vulnerabilities",
                "description": "Prioritize vulnerable dependencies from OSV-Scanner/Trivy reports with changed-import evidence. Read-only: never runs scanners; provide pre-generated files, never queries registries or the network.",
                "annotations": { "title": "Prioritize vulnerabilities", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "osv": { "type": "array", "items": { "type": "string" }, "maxItems": 32, "description": "Root-relative OSV-Scanner files." },
                        "trivy": { "type": "array", "items": { "type": "string" }, "maxItems": 32, "description": "Root-relative Trivy files." },
                        "baseline_osv": { "type": "array", "items": { "type": "string" }, "maxItems": 32, "description": "Root-relative baseline OSV files." },
                        "baseline_trivy": { "type": "array", "items": { "type": "string" }, "maxItems": 32, "description": "Root-relative baseline Trivy files." },
                        "base": { "type": "string", "description": "Git base revision for changed-import evidence. Exclusive with staged/target." },
                        "staged": { "type": "boolean", "default": false, "description": "Compare against the index. Exclusive with base/target." },
                        "target": { "type": "string", "description": "Revision to compare against base. Exclusive with base/staged." },
                        "minimum_severity": { "type": "string", "enum": ["low", "medium", "high", "critical"], "description": "Gate floor; absent means informational." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Keep at most this many findings." },
                    },
                },
            },
            {
                "name": "sql_risks",
                "description": "Flag static PostgreSQL query risks in .sql files and host-language call sites. Read-only: never executes SQL or connects to a database.",
                "annotations": { "title": "Flag SQL risks", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "large_offset": { "type": "integer", "minimum": 1, "description": "Fail numeric OFFSET past this count." },
                        "migration_roots": { "type": "array", "items": { "type": "string" }, "maxItems": 32, "description": "Root-relative migration directories declaring tables." },
                        "minimum_severity": { "type": "string", "enum": ["low", "medium", "high", "critical"], "description": "Gate floor; absent means informational." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Keep at most this many findings." },
                    },
                },
            },
            {
                "name": "test_targets",
                "description": "Use to decide what to test: ranks functions holding uncovered decision lines by CRAP. Requires coverage.",
                "annotations": { "title": "Rank test targets", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "coverage": { "type": "string", "description": "Required coverage file path (.info for LCOV, .xml for JaCoCo). Line coverage required; branch records (BRDA, mb/cb) improve ratios when present." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Keep at most this many targets." },
                    },
                    "required": ["coverage"],
                },
            },
            {
                "name": "dependencies",
                "description": "Static dependency graph for a path: per-file fan-in/fan-out, edges, cycles, and unresolved imports. Use impact for one target's blast radius.",
                "annotations": { "title": "Dependency graph", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." }
                    }
                },
            },
            {
                "name": "impact",
                "description": "Transitive dependents and blast radius for one file. The target must exist under path. Use dependencies for the whole graph.",
                "annotations": { "title": "Impact blast radius", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "description": "Target file path relative to path." },
                        "path": { "type": "string", "default": "." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20, "description": "Keep at most this many dependents." }
                    },
                    "required": ["target"]
                },
            },
            {
                "name": "coupling",
                "description": "Files that historically change together with one target (`git log` co-changes). Use before editing a target to find hidden contracts the static graph cannot see.",
                "annotations": { "title": "Change coupling", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "description": "Target file path relative to path." },
                        "path": { "type": "string", "default": "." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "default": 20, "description": "Keep at most this many related files." },
                        "min_cochanges": { "type": "integer", "minimum": 1, "default": 2, "description": "Ignore pairs that co-changed fewer times." }
                    },
                    "required": ["target"]
                },
            },
            {
                "name": "hotspots",
                "description": "Rank files by churn, complexity, and CRAP. Use for a first look at a repository; use repo_summary for a flat function list. Coverage adds the CRAP dimension.",
                "annotations": { "title": "Churn hotspots", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "default": 10, "description": "Keep at most this many hotspots." },
                        "since": { "type": "string", "enum": ["30d", "90d", "365d"], "default": "90d", "description": "Git history window for churn." },
                        "coverage": { "type": ["string", "null"], "description": "Coverage file path (.info for LCOV, .xml for JaCoCo)." }
                    }
                }
            },
            {
                "name": "duplication",
                "description": "Token-clone detection. With base, reports new/existing/resolved drift against that revision; without it, one current-state report.",
                "annotations": { "title": "Duplication", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "base": { "type": "string", "description": "Git base revision for new/existing/resolved drift." }
                    }
                }
            },
            {
                "name": "policy",
                "description": "Architecture-rule evaluation from leadline.toml. With base, reports new/existing/resolved violations; without it, one current-state evaluation.",
                "annotations": { "title": "Architecture policy", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "base": { "type": "string", "description": "Git base revision for policy drift." }
                    }
                }
            },
            {
                "name": "risk",
                "description": "Explainable change-risk ranking: score plus complexity, CRAP, churn, impact, ownership, and policy components. Coverage enables the CRAP component.",
                "annotations": { "title": "Change risk", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "top": { "type": "integer", "minimum": 1, "maximum": 200, "default": 10, "description": "Keep at most this many ranked files." },
                        "since": { "type": "string", "enum": ["30d", "90d", "365d"], "default": "90d", "description": "Git history window for churn." },
                        "coverage": { "type": ["string", "null"], "description": "Coverage file path (.info for LCOV, .xml for JaCoCo)." }
                    }
                }
            },
            {
                "name": "debt",
                "description": "Full-state comparison against a base revision: threshold findings and risk-score changes, with new/existing/resolved status. Reports state; use check to gate.",
                "annotations": { "title": "Debt comparison", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "base": { "type": "string", "default": "HEAD~1" },
                        "target": { "type": "string", "default": "worktree", "description": "'worktree', 'index', or a revision to compare against base." },
                        "renames": { "type": "boolean", "default": false, "description": "Detect Git file renames and pair old-path content with new-path content." },
                        "since": { "type": "string", "enum": ["30d", "90d", "365d"], "default": "90d" }
                    }
                }
            },
            {
                "name": "project",
                "description": "Canonical project build: summary KPIs, top risk rows, and optional mutation/test-map inputs. The heaviest tool; prefer targeted tools first.",
                "annotations": { "title": "Project summary", "readOnlyHint": true, "idempotentHint": true, "openWorldHint": false },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "default": "." },
                        "target": { "type": "string", "description": "Git revision to snapshot instead of the worktree." },
                        "since": { "type": "string", "enum": ["30d", "90d", "365d"], "default": "90d" },
                        "coverage": { "type": ["string", "null"], "description": "Coverage file path (.info for LCOV, .xml for JaCoCo)." },
                        "ownership": { "type": "string", "enum": ["aggregate", "include_authors", "anonymize_authors"], "default": "aggregate" },
                        "pit": { "type": "array", "items": { "type": "string" }, "description": "PIT mutation report paths (root-relative, inside the working directory)." },
                        "stryker": { "type": "array", "items": { "type": "string" }, "description": "Stryker mutation report paths (root-relative, inside the working directory)." },
                        "test_map": { "type": "array", "items": { "type": "string" }, "description": "Test-map report paths (root-relative, inside the working directory)." },
                        "snapshots": { "type": "string", "description": "Snapshot store path (root-relative, inside the working directory)." }
                    }
                }
            },
        ],
    })
}

fn success_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: serde_json::Value, code: i64, message: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

/// One-error response for a batch that cannot be processed. Batch requests
/// keep the array shape so clients that posted a batch can parse the reply.
fn batch_error_response(code: i64, message: &str) -> String {
    serde_json::Value::Array(vec![error_response(serde_json::Value::Null, code, message)])
        .to_string()
}

fn parse_error_response() -> String {
    error_response(serde_json::Value::Null, -32700, "Parse error").to_string()
}

/// Optional string argument: absent or null when unset; any other type is an
/// invalid-params error instead of silently reading as absent.
fn opt_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<Option<&'a str>, (i64, String)> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or((-32602, format!("{key} must be a string"))),
    }
}

/// Internal generated-row accessor; never reads user parameters.
fn row_str<'a>(row: &'a serde_json::Value, key: &str) -> &'a str {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}

fn req_str<'a>(params: &'a serde_json::Value, key: &str) -> Result<&'a str, (i64, String)> {
    opt_str(params, key)?.ok_or((-32602, format!("missing required string param '{key}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_header_rules_require_a_loopback_authority_when_bound_locally() {
        assert!(host_allowed(Some("localhost"), true));
        assert!(host_allowed(Some("127.0.0.1:3000"), true));
        assert!(host_allowed(Some("[::1]:3000"), true));
        assert!(host_allowed(Some("127.0.0.2"), true));
        assert!(!host_allowed(Some("evil.example"), true));
        assert!(!host_allowed(None, true));
        assert!(!host_allowed(Some(""), true));
        // A wildcard-bound listener answers whatever address was reached.
        assert!(host_allowed(Some("leadline.internal"), false));
    }

    #[test]
    fn loopback_detection_covers_ipv4_mapped_addresses() {
        assert!(is_loopback_ip("127.0.0.1".parse().unwrap()));
        assert!(is_loopback_ip("::1".parse().unwrap()));
        assert!(is_loopback_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_loopback_ip("192.0.2.1".parse().unwrap()));
    }

    #[test]
    fn expired_read_deadlines_fail_with_408() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        let past = std::time::Instant::now() - std::time::Duration::from_millis(1);
        assert_eq!(arm_read_deadline(&server, past).unwrap_err().0, 408);
        let future = std::time::Instant::now() + std::time::Duration::from_secs(1);
        assert!(arm_read_deadline(&server, future).is_ok());
        drop(client);
    }

    #[test]
    fn jsonrpc_ids_reject_structured_values() {
        assert!(valid_id(&serde_json::json!("1")));
        assert!(valid_id(&serde_json::json!(1)));
        assert!(valid_id(&serde_json::Value::Null));
        assert!(!valid_id(&serde_json::json!(true)));
        assert!(!valid_id(&serde_json::json!([1])));
        assert!(!valid_id(&serde_json::json!({"a": 1})));
    }

    #[test]
    fn expired_write_deadlines_fail() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut writer = DeadlineWriter {
            stream: &server,
            deadline: std::time::Instant::now() - std::time::Duration::from_millis(1),
        };
        use std::io::Write as _;
        assert!(writer.write_all(b"x").is_err());
        drop(client);
    }

    #[test]
    fn body_reservations_enforce_the_global_ceiling() {
        let first = reserve_body_bytes(MAX_INFLIGHT_BODY_BYTES).unwrap();
        assert_eq!(reserve_body_bytes(1).unwrap_err().0, 503);
        drop(first);
        // Releasing restores the budget for later requests.
        assert!(reserve_body_bytes(MAX_INFLIGHT_BODY_BYTES).is_ok());
    }

    #[test]
    fn path_arguments_stay_raw_for_the_caller_to_resolve() {
        let params = serde_json::json!({ "roots": ["migrations", "a/../b"] });
        // Shape validation keeps the raw text: migration roots must reach
        // normalize_migration_root unchanged so its `..` rejection applies.
        assert_eq!(
            path_arguments(&params, "t", "roots", false).unwrap(),
            vec!["migrations".to_owned(), "a/../b".to_owned()]
        );
        // Artifact paths normalize interior parents and reject escapes.
        assert_eq!(relative_path("t", "roots", "a/../b").unwrap(), "b");
        assert!(relative_path("t", "roots", "../escape").is_err());
        assert!(relative_path("t", "roots", "/abs").is_err());
    }

    #[test]
    fn dispatch_method_covers_known_methods_and_rejects_unknown() {
        assert!(dispatch_method("ping", &serde_json::Value::Null).is_ok());
        assert_eq!(
            dispatch_method("does/not/exist", &serde_json::Value::Null)
                .unwrap_err()
                .0,
            -32601
        );
    }

    #[test]
    fn stdio_lines_are_bounded_and_arrays_are_rejected_for_direct_tools() {
        let mut reader = std::io::BufReader::new(&b"{\"id\":1}\n"[..]);
        assert_eq!(
            read_request_line(&mut reader, 64).unwrap().as_deref(),
            Some("{\"id\":1}\n")
        );
        assert_eq!(read_request_line(&mut reader, 64).unwrap(), None);
        // A line past the limit fails instead of allocating the rest.
        let mut reader = std::io::BufReader::new(&b"aaaaaaaaaaaaaaaaaaaa\n"[..]);
        assert!(read_request_line(&mut reader, 8).is_err());
        // Direct tool methods take named parameters only, so an array must
        // not silently analyze the default path.
        assert_eq!(
            dispatch_method("analyze", &serde_json::json!([1]))
                .unwrap_err()
                .0,
            -32602
        );
    }

    #[test]
    fn cached_index_reuses_the_parsed_index_until_the_file_changes() {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "leadline-mcp-index-cache-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut first = crate::index::AnalysisIndex::empty(".", "none");
        first.scope = "first".to_owned();
        first.save(&dir).unwrap();
        let a = cached_index(&dir);
        let b = cached_index(&dir);
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.scope, "first");

        // A rewrite with a different length must invalidate on its own; the
        // test never sleeps, so a coarse mtime granularity cannot hide it.
        let mut second = crate::index::AnalysisIndex::empty(".", "none");
        second.scope = "second-with-a-longer-scope".to_owned();
        second.save(&dir).unwrap();
        let c = cached_index(&dir);
        assert!(!Arc::ptr_eq(&a, &c));
        assert_eq!(c.scope, "second-with-a-longer-scope");

        // A deleted index must not keep serving the last parsed value.
        std::fs::remove_file(dir.join(crate::index::INDEX_FILE_NAME)).unwrap();
        let d = cached_index(&dir);
        assert!(!Arc::ptr_eq(&c, &d));
        assert!(d.files.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
