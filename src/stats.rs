//! Local metrics server: the canonical `Project` model over loopback.
//!
//! `stats` analyzes once at launch, keeps the serialized `Project`, and
//! answers every request from that snapshot. `POST /api/refresh` re-analyzes
//! on its own connection under a single-flight guard and swaps the snapshot
//! only after a complete build, so a failed refresh leaves the previous data
//! served. The transport, including the `Host`/`Origin` guards, lives in
//! [`crate::http`]; routing and the embedded page live here.

use crate::analytics::ProjectRequest;
use crate::http::{DEFAULT_HOST, DEFAULT_PORT, JSON_CONTENT_TYPE, Request, Response, parse_port};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The React dashboard is built into `web/dist` (`npm run build` in
/// `web/`, checked in) and embedded here: the binary serves a complete UI
/// with nothing beside it. Asset names are hash-free by Vite config so
/// these paths stay stable across rebuilds.
const INDEX_HTML: &str = include_str!("../web/dist/index.html");
const APP_JS: &str = include_str!("../web/dist/assets/app.js");
const APP_CSS: &str = include_str!("../web/dist/assets/index.css");
const LOGO_SVG: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.svg"));

const HTML_CONTENT_TYPE: &str = "text/html";
const CSS_CONTENT_TYPE: &str = "text/css";
const JS_CONTENT_TYPE: &str = "text/javascript";
const SVG_CONTENT_TYPE: &str = "image/svg+xml";

/// Every path the route table answers, so a wrong method on one of them
/// answers 405 instead of 404.
const KNOWN_PATHS: [&str; 8] = [
    "/",
    "/assets/app.js",
    "/assets/index.css",
    "/logo.svg",
    "/api/project",
    "/api/telemetry",
    "/api/refresh",
    "/health",
];

/// `stats` bind and browser options.
pub struct ServeOptions {
    pub host: String,
    pub port: u16,
    pub open: bool,
}

/// Splits `--port [N]`, `--host ADDR`, and `--open` out of `args`, returning
/// the flags that remain for the shared analysis parser.
///
/// A bare `--port` means [`DEFAULT_PORT`], and `--port 0` asks the OS for a
/// free port.
pub fn parse_serve_flags(args: &[String]) -> Result<(Vec<String>, ServeOptions), String> {
    let mut rest = Vec::new();
    let mut host = DEFAULT_HOST.to_owned();
    let mut port = DEFAULT_PORT;
    let mut open = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--port" => match args.get(index + 1).map(String::as_str) {
                Some(next) if !next.starts_with('-') => {
                    port = parse_port(next)?;
                    index += 1;
                }
                _ => port = DEFAULT_PORT,
            },
            "--host" => {
                index += 1;
                host = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "--host requires an address".to_owned())?;
            }
            "--open" => open = true,
            other => rest.push(other.to_owned()),
        }
        index += 1;
    }
    Ok((rest, ServeOptions { host, port, open }))
}

/// Analysis failure before the server starts. `main` maps it to the CLI's
/// incomplete-analysis exit code instead of an internal error.
#[derive(Debug)]
pub struct AnalysisError(String);

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for AnalysisError {}

/// Serves until the process is interrupted.
pub fn serve(request: ProjectRequest, options: ServeOptions) -> crate::Result<()> {
    let (listener, port, fell_back) = crate::http::bind(&options.host, options.port)?;
    if fell_back {
        eprintln!(
            "leadline: port {} in use, using free port {port}",
            options.port
        );
    }
    let server = match Server::started(request) {
        Ok(server) => server,
        Err(error) => return Err(Box::new(AnalysisError(error.to_string()))),
    };
    let url = format!("http://{}:{port}", options.host);
    eprintln!("leadline: stats on {url}");
    if options.open {
        open_browser(&url);
    }
    crate::http::serve(listener, move |request| server.route(request));
    Ok(())
}

/// Opens the page in the default browser, best effort: a failure only warns.
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    if let Err(error) = std::process::Command::new(program).arg(url).spawn() {
        eprintln!("leadline: cannot open {url}: {error}");
    }
}

/// One complete analysis, serialized once.
struct Snapshot {
    bytes: Vec<u8>,
    analyzed_at: u64,
    duration_ms: u128,
    head_commit: Option<String>,
}

impl Snapshot {
    /// The metadata `/health` and `/api/refresh` both report.
    fn meta(&self) -> serde_json::Value {
        serde_json::json!({
            "analyzed_at": self.analyzed_at,
            "duration_ms": self.duration_ms,
            "head_commit": self.head_commit,
            "analyzer_version": env!("CARGO_PKG_VERSION"),
        })
    }
}

struct Server {
    request: ProjectRequest,
    snapshot: std::sync::RwLock<Arc<Snapshot>>,
    refreshing: AtomicBool,
    telemetry: Arc<std::sync::Mutex<std::collections::VecDeque<Sample>>>,
}

/// One telemetry sample: the sampler's latest CPU/RSS readings, store-wide
/// totals, and per-operation p90 costs, served as the `series` array.
#[derive(Clone, Debug, serde::Serialize)]
struct Sample {
    t_ms: u64,
    cpu_mc: Option<u64>,
    rss_bytes: Option<u64>,
    invocations: u64,
    findings: u64,
    ops: Vec<crate::telemetry::OpCost>,
}

/// Seconds between samples; 720 samples cover one hour.
const SAMPLE_EVERY: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_SAMPLES: usize = 720;

/// Samples the telemetry store into a capped ring, best effort: a poisoned
/// lock still serves, and a missing store reads as zeros and nulls. The CPU
/// and memory come from the sampler's live gauges — what real invocations
/// consumed — never from this idling server process.
fn spawn_sampler(samples: Arc<std::sync::Mutex<std::collections::VecDeque<Sample>>>) {
    std::thread::spawn(move || {
        loop {
            let directory = std::env::var_os(crate::telemetry::METRICS_DIR_VAR)
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from);
            // One store read per tick: the sampler needs gauges, totals,
            // and p90 costs together.
            let sample = directory
                .as_deref()
                .map(|dir| crate::telemetry::sample_store(dir, 12))
                .unwrap_or_default();
            push_sample(
                &samples,
                Sample {
                    t_ms: unix_millis(),
                    cpu_mc: sample.cpu_mc,
                    rss_bytes: sample.rss_bytes,
                    invocations: sample.invocations,
                    findings: sample.findings,
                    ops: sample.ops,
                },
            );
            std::thread::sleep(SAMPLE_EVERY);
        }
    });
}

/// Pushes one sample, dropping the oldest past the cap.
fn push_sample(
    samples: &Arc<std::sync::Mutex<std::collections::VecDeque<Sample>>>,
    sample: Sample,
) {
    let mut ring = samples.lock().unwrap_or_else(|error| error.into_inner());
    if ring.len() >= MAX_SAMPLES {
        ring.pop_front();
    }
    ring.push_back(sample);
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

impl Server {
    /// Analyzes once, so every request is answered from a snapshot.
    fn started(request: ProjectRequest) -> crate::Result<Self> {
        let snapshot = Self::analyze(&request)?;
        let telemetry = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        spawn_sampler(Arc::clone(&telemetry));
        Ok(Self {
            request,
            snapshot: std::sync::RwLock::new(Arc::new(snapshot)),
            refreshing: AtomicBool::new(false),
            telemetry,
        })
    }

    /// Builds the canonical model and serializes it once per analysis.
    fn analyze(request: &ProjectRequest) -> crate::Result<Snapshot> {
        let started = std::time::Instant::now();
        let project = crate::analytics::build(request)?;
        // Pretty JSON plus a trailing newline: byte-identical to
        // `project --json`, which prints through the same encoding.
        let mut bytes = serde_json::to_vec_pretty(&project)?;
        bytes.push(b'\n');
        let snapshot = Snapshot {
            bytes,
            analyzed_at: unix_millis() / 1000,
            duration_ms: started.elapsed().as_millis(),
            head_commit: project.meta.head_commit,
        };
        Ok(snapshot)
    }

    /// The current snapshot; a poisoned lock still serves the last swap.
    fn snapshot(&self) -> Arc<Snapshot> {
        let current = self
            .snapshot
            .read()
            .unwrap_or_else(|error| error.into_inner());
        Arc::clone(&current)
    }

    fn route(&self, request: &Request) -> Response {
        let path = request.path.split('?').next().unwrap_or(&request.path);
        match (request.method.as_str(), path) {
            ("GET", "/") => (200, HTML_CONTENT_TYPE, INDEX_HTML.as_bytes().to_vec()),
            ("GET", "/assets/app.js") => (200, JS_CONTENT_TYPE, APP_JS.as_bytes().to_vec()),
            ("GET", "/assets/index.css") => (200, CSS_CONTENT_TYPE, APP_CSS.as_bytes().to_vec()),
            ("GET", "/logo.svg") => (200, SVG_CONTENT_TYPE, LOGO_SVG.as_bytes().to_vec()),
            ("GET", "/api/project") => (200, JSON_CONTENT_TYPE, self.snapshot().bytes.clone()),
            ("GET", "/api/telemetry") => {
                let mut body = crate::telemetry::snapshot_json();
                if let Some(map) = body.as_object_mut() {
                    let series: Vec<Sample> = self
                        .telemetry
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .iter()
                        .cloned()
                        .collect();
                    map.insert(
                        "series".to_owned(),
                        serde_json::to_value(&series).unwrap_or_default(),
                    );
                }
                (200, JSON_CONTENT_TYPE, body.to_string().into_bytes())
            }
            ("GET", "/health") => {
                let mut body = self.snapshot().meta();
                body["status"] = "ok".into();
                (200, JSON_CONTENT_TYPE, body.to_string().into_bytes())
            }
            ("POST", "/api/refresh") => self.refresh(),
            (_, known) if KNOWN_PATHS.contains(&known) => (
                405,
                JSON_CONTENT_TYPE,
                crate::http::http_error(&format!(
                    "method {} not allowed on {known}",
                    request.method
                ))
                .into_bytes(),
            ),
            _ => (
                404,
                JSON_CONTENT_TYPE,
                crate::http::http_error("not found").into_bytes(),
            ),
        }
    }

    /// Re-analyzes on this connection and swaps the snapshot only on success.
    fn refresh(&self) -> Response {
        let Some(_guard) = RefreshGuard::acquire(&self.refreshing) else {
            return (
                409,
                JSON_CONTENT_TYPE,
                crate::http::http_error("refresh already running").into_bytes(),
            );
        };
        match Self::analyze(&self.request) {
            Ok(snapshot) => {
                let body = serde_json::json!({
                    "status": "ok",
                    "meta": snapshot.meta(),
                })
                .to_string();
                *self
                    .snapshot
                    .write()
                    .unwrap_or_else(|error| error.into_inner()) = Arc::new(snapshot);
                (200, JSON_CONTENT_TYPE, body.into_bytes())
            }
            Err(error) => {
                eprintln!("leadline: refresh failed: {error}");
                (
                    500,
                    JSON_CONTENT_TYPE,
                    crate::http::http_error(&format!("refresh failed: {error}")).into_bytes(),
                )
            }
        }
    }
}

/// Single-flight guard for refresh: `acquire` fails while another refresh
/// runs, and `Drop` clears the flag on every path, early returns included.
struct RefreshGuard<'a>(&'a AtomicBool);

impl<'a> RefreshGuard<'a> {
    /// `None` while another refresh holds the flag.
    ///
    /// The guard is built only on the winning branch: `then_some` would build
    /// one eagerly on every attempt, and dropping that temporary would clear
    /// the flag a running refresh still holds.
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        match flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => Some(Self(flag)),
            Err(_) => None,
        }
    }
}

impl Drop for RefreshGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_flags_default_to_loopback_port_3000() {
        let (rest, options) = parse_serve_flags(&["src".to_owned()]).unwrap();
        assert_eq!(options.host, "127.0.0.1");
        assert_eq!(options.port, 3000);
        assert!(!options.open);
        assert_eq!(rest, vec!["src".to_owned()]);
    }

    #[test]
    fn bare_port_and_host_and_open_are_parsed() {
        let (rest, options) = parse_serve_flags(&[
            "--port".to_owned(),
            "--host".to_owned(),
            "0.0.0.0".to_owned(),
            "--open".to_owned(),
        ])
        .unwrap();
        assert_eq!(options.port, 3000);
        assert_eq!(options.host, "0.0.0.0");
        assert!(options.open);
        assert!(rest.is_empty());
    }

    #[test]
    fn serve_flag_falls_through_to_the_analysis_parser() {
        // `--serve` is not a serve flag; it stays in `rest` for the shared
        // analysis parser, which rejects it as an unknown stats option.
        let (rest, _) = parse_serve_flags(&["--serve".to_owned()]).unwrap();
        assert_eq!(rest, vec!["--serve".to_owned()]);
    }

    #[test]
    fn analysis_flags_and_path_keep_their_order() {
        let (rest, options) = parse_serve_flags(&[
            "src".to_owned(),
            "--port".to_owned(),
            "0".to_owned(),
            "--lcov".to_owned(),
            "lcov.info".to_owned(),
            "--open".to_owned(),
        ])
        .unwrap();
        assert_eq!(options.port, 0);
        assert!(options.open);
        assert_eq!(
            rest,
            vec![
                "src".to_owned(),
                "--lcov".to_owned(),
                "lcov.info".to_owned()
            ]
        );
    }

    /// The route table and `KNOWN_PATHS` cannot drift: every known path
    /// answers a method status instead of 404, and unknown paths stay 404.
    #[test]
    fn known_paths_match_the_route_table() {
        let server = test_server();
        let request = |method: &str, path: &str| Request {
            method: method.to_owned(),
            path: path.to_owned(),
            body: Vec::new(),
        };
        for path in KNOWN_PATHS {
            let (status, _, _) = server.route(&request("GET", path));
            assert_ne!(status, 404, "{path}");
        }
        let (status, _, _) = server.route(&request("POST", "/api/project"));
        assert_eq!(status, 405);
        let (status, _, _) = server.route(&request("GET", "/nope"));
        assert_eq!(status, 404);
    }

    /// A refresh whose analysis fails answers 500 and leaves the previous
    /// snapshot served, byte for byte.
    #[test]
    fn a_failed_refresh_answers_500_and_keeps_the_previous_snapshot() {
        let dir =
            std::env::temp_dir().join(format!("leadline-stats-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("one.rs"), "fn one() {}\n").unwrap();
        let mut request = test_server().request;
        request.path = dir.clone();
        let server = Server::started(request).expect("the fixture analyzes");
        let before = server.snapshot();
        std::fs::remove_dir_all(&dir).unwrap();

        let refresh = Request {
            method: "POST".to_owned(),
            path: "/api/refresh".to_owned(),
            body: Vec::new(),
        };
        let (status, content_type, body) = server.route(&refresh);
        assert_eq!(status, 500);
        assert_eq!(content_type, JSON_CONTENT_TYPE);
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            error["error"]
                .as_str()
                .is_some_and(|message| message.starts_with("refresh failed:")),
            "unexpected body: {error}"
        );

        let project = Request {
            method: "GET".to_owned(),
            path: "/api/project".to_owned(),
            body: Vec::new(),
        };
        let (status, _, served) = server.route(&project);
        assert_eq!(status, 200);
        assert_eq!(served, before.bytes);
    }

    /// A failed attempt must not clear the flag the winner still holds:
    /// `then_some` built a guard eagerly here and dropping it released the
    /// lock, which let parallel refreshes run against the same snapshot.
    #[test]
    fn a_losing_acquire_leaves_the_flag_held() {
        let flag = AtomicBool::new(false);
        let guard = RefreshGuard::acquire(&flag).expect("first refresh acquires");
        assert!(RefreshGuard::acquire(&flag).is_none());
        assert!(
            RefreshGuard::acquire(&flag).is_none(),
            "the failed attempt released the held flag"
        );
        drop(guard);
        assert!(RefreshGuard::acquire(&flag).is_some());
    }

    /// A refresh that is already running answers 409 without touching the
    /// analysis, and dropping the guard lets the next refresh start.
    #[test]
    fn a_second_refresh_while_one_runs_answers_409() {
        let server = test_server();
        let request = Request {
            method: "POST".to_owned(),
            path: "/api/refresh".to_owned(),
            body: Vec::new(),
        };
        let guard = RefreshGuard::acquire(&server.refreshing).expect("first refresh acquires");
        let (status, content_type, body) = server.route(&request);
        assert_eq!(status, 409);
        assert_eq!(content_type, JSON_CONTENT_TYPE);
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"], "refresh already running");
        drop(guard);
        assert!(RefreshGuard::acquire(&server.refreshing).is_some());
    }

    /// A failed launch analysis surfaces as [`AnalysisError`], which `main`
    /// maps to the incomplete-analysis exit code rather than an internal one.
    #[test]
    fn a_failed_launch_analysis_is_an_analysis_error() {
        let mut request = test_server().request;
        request.path = std::path::PathBuf::from("/nonexistent-leadline-stats");
        let error = serve(
            request,
            ServeOptions {
                host: DEFAULT_HOST.to_owned(),
                port: 0,
                open: false,
            },
        )
        .expect_err("a missing path cannot be analyzed");
        assert!(
            error.downcast_ref::<AnalysisError>().is_some(),
            "expected an AnalysisError, got {error}"
        );
    }

    fn test_server() -> Server {
        Server {
            request: ProjectRequest {
                path: std::path::PathBuf::from("."),
                target: crate::source_snapshot::SnapshotTarget::Worktree,
                window: crate::history::HistoryWindow::Days90,
                mutation_inputs: Vec::new(),
                test_maps: Vec::new(),
                ownership_mode: crate::ownership::OwnershipMode::AggregateOnly,
                snapshots_path: None,
                coverage: None,
            },
            snapshot: std::sync::RwLock::new(Arc::new(Snapshot {
                bytes: b"{}".to_vec(),
                analyzed_at: 0,
                duration_ms: 0,
                head_commit: None,
            })),
            refreshing: AtomicBool::new(false),
            telemetry: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
        }
    }

    /// The sampler ring keeps the newest `MAX_SAMPLES`: pushing past the cap
    /// drops the oldest first.
    #[test]
    fn sampler_ring_drops_the_oldest_past_the_cap() {
        let ring = Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
        for index in 0..(MAX_SAMPLES + 10) {
            push_sample(
                &ring,
                Sample {
                    t_ms: index as u64,
                    cpu_mc: None,
                    rss_bytes: None,
                    invocations: index as u64,
                    findings: 0,
                    ops: Vec::new(),
                },
            );
        }
        let ring = ring.lock().unwrap();
        assert_eq!(ring.len(), MAX_SAMPLES);
        assert_eq!(ring.front().map(|sample| sample.invocations), Some(10));
        assert_eq!(
            ring.back().map(|sample| sample.invocations),
            Some((MAX_SAMPLES + 9) as u64)
        );
    }
}
