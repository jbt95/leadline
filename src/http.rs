//! Hardened HTTP/1.1 transport shared by the MCP and `stats` servers.
//!
//! One `Connection: close` code path: request line and headers under fixed
//! ceilings, a body read under one whole-request deadline and a process-wide
//! reservation, a fixed connection ceiling, `Host`/`Origin` validation against
//! DNS rebinding, and a response written under one deadline. A handler sees a
//! parsed [`Request`] and returns `(status, content_type, body)`.

/// Port used when a caller asks for the default port.
pub const DEFAULT_PORT: u16 = 3000;

/// Bind address used when no host is given (loopback only).
pub const DEFAULT_HOST: &str = "127.0.0.1";

/// Parse a `--port` value; shared by the MCP and `stats` servers so the
/// error reads the same on both. A bare `--port` means [`DEFAULT_PORT`],
/// and `--port 0` asks the OS for a free port.
pub(crate) fn parse_port(raw: &str) -> Result<u16, String> {
    raw.parse::<u16>()
        .map_err(|_| format!("invalid --port '{raw}': expected 0-65535"))
}

/// Largest request body accepted (32 MiB); larger reads fail with 413
/// instead of growing a buffer without bound.
pub(crate) const MAX_HTTP_BODY: usize = 32 * 1024 * 1024;

/// Content type of every JSON body the transport and its callers write.
pub(crate) const JSON_CONTENT_TYPE: &str = "application/json";

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

/// One parsed HTTP request, as a handler sees it.
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

/// A parsed request plus the transport-level state a handler never sees: the
/// authorities the guards validate, and the reservation that keeps the body
/// counted against the process-wide buffering ceiling.
struct ParsedRequest {
    request: Request,
    host: Option<String>,
    origin: Option<String>,
    reservation: BodyReservation,
}

impl ParsedRequest {
    /// Split off the handler-facing request. The reservation travels back so
    /// the caller holds it for as long as the body is alive.
    fn into_request(self) -> (Request, BodyReservation) {
        (self.request, self.reservation)
    }
}

/// What a handler answers with: `(status, content_type, body)`.
pub type Response = (u16, &'static str, Vec<u8>);

/// Bind for HTTP mode, falling back to an OS-assigned free port when the
/// requested one is taken. Returns the listener, the actual port, and
/// whether fallback happened (so callers can report it).
pub fn bind(host: &str, port: u16) -> std::io::Result<(std::net::TcpListener, u16, bool)> {
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

/// Serve an already-bound listener: one worker per connection up to a fixed
/// ceiling, then fail fast with 503 instead of growing threads.
///
/// The guards run here, before `handler` sees anything. HTTP/1.1 requires
/// exactly one `Host`, and a loopback-bound listener only accepts loopback
/// authorities (400 otherwise). MCP Streamable HTTP requires `Origin`
/// validation against DNS rebinding: a browser origin must be loopback, and
/// anything else is 403. Non-browser clients omit `Origin` and pass.
pub fn serve<H>(listener: std::net::TcpListener, handler: H)
where
    H: Fn(&Request) -> Response + Send + Sync + 'static,
{
    let limiter = std::sync::Arc::new(ConnectionLimiter::default());
    let handler = std::sync::Arc::new(handler);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => match limiter.try_acquire() {
                Some(permit) => {
                    let handler = std::sync::Arc::clone(&handler);
                    std::thread::spawn(move || {
                        handle_connection(stream, &*handler);
                        drop(permit);
                    });
                }
                None => {
                    crate::telemetry::record_mcp_error("http", "http_busy");
                    write_response(
                        stream,
                        503,
                        JSON_CONTENT_TYPE,
                        http_error("too many concurrent connections").as_bytes(),
                    );
                }
            },
            Err(error) => eprintln!("leadline: connection failed: {error}"),
        }
    }
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

fn handle_connection<H>(stream: std::net::TcpStream, handler: &H)
where
    H: Fn(&Request) -> Response,
{
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
    let (status, content_type, body) = match read_request(&stream, deadline) {
        Ok(parsed) => {
            if !host_allowed(parsed.host.as_deref(), loopback_only) {
                (
                    400,
                    JSON_CONTENT_TYPE,
                    http_error("invalid Host header").into_bytes(),
                )
            } else if !origin_allowed(parsed.origin.as_deref()) {
                (
                    403,
                    JSON_CONTENT_TYPE,
                    http_error("origin not allowed").into_bytes(),
                )
            } else {
                let (request, _reservation) = parsed.into_request();
                handler(&request)
            }
        }
        Err((status, body)) => (status, JSON_CONTENT_TYPE, body.into_bytes()),
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
    write_response(stream, status, content_type, &body);
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
) -> Result<ParsedRequest, (u16, String)> {
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
    Ok(ParsedRequest {
        request: Request { method, path, body },
        host,
        origin,
        reservation,
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

pub(crate) fn bad_request(message: &str) -> (u16, String) {
    (400, http_error(message))
}

pub(crate) fn http_error(message: &str) -> String {
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

fn write_response(stream: std::net::TcpStream, status: u16, content_type: &str, body: &[u8]) {
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
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = writer.write_all(body);
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
}
