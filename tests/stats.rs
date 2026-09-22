use serde_json::Value;
use std::path::PathBuf;

/// A spawned `stats` server over a temporary committed repository, killed
/// and cleaned up when the guard drops.
struct StatsServer {
    child: std::process::Child,
    root: PathBuf,
    port: u16,
}

impl StatsServer {
    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for StatsServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Builds a tiny repository with one commit, starts the real server on an
/// OS-assigned port, and reads the port out of its banner.
fn spawn_stats_server() -> StatsServer {
    use std::io::BufRead as _;
    let root = std::env::temp_dir().join(format!(
        "leadline-stats-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(root.join("one.rs"), "fn one() {}\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "base"]);
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["stats", "--port", "0", root.to_str().unwrap()])
        // The server runs with the ambient local-metrics store removed, so a
        // test run cannot write into the developer's store.
        .env_remove("LEADLINE_METRICS_DIR")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut banner = String::new();
    reader.read_line(&mut banner).unwrap();
    let port = banner
        .rsplit(':')
        .next()
        .and_then(|tail| tail.split('/').next())
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or_else(|| panic!("no listening port in {banner:?}"));
    // Keep draining stderr so a chatty server never blocks on a full pipe.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });
    StatsServer { child, root, port }
}

/// Sends one raw request and returns the status, content type, and body.
fn exchange(port: u16, head: &str) -> (u16, String, String) {
    use std::io::{Read as _, Write as _};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let request = format!("{head}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let content_type = lines
        .find_map(|line| line.strip_prefix("Content-Type: "))
        .unwrap()
        .to_owned();
    (status, content_type, body.to_owned())
}

#[test]
fn stats_serves_the_dashboard_and_assets() {
    let server = spawn_stats_server();
    let port = server.port();
    for (path, content_type) in [
        ("/", "text/html"),
        ("/assets/app.js", "text/javascript"),
        ("/assets/index.css", "text/css"),
        ("/logo.svg", "image/svg+xml"),
    ] {
        let (status, served, body) =
            exchange(port, &format!("GET {path} HTTP/1.1\r\nHost: localhost"));
        assert_eq!(status, 200, "{path}");
        assert_eq!(served, content_type, "{path}");
        assert!(!body.is_empty(), "{path}");
    }
}

#[test]
fn stats_serves_canonical_json() {
    let server = spawn_stats_server();
    let port = server.port();

    let (status, content_type, body) =
        exchange(port, "GET /api/project HTTP/1.1\r\nHost: localhost");
    assert_eq!(status, 200);
    assert_eq!(content_type, "application/json");
    let project: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(project["meta"]["schema_version"], 1);

    // `/api/project` is byte-identical to `project --json` on the same tree.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .args(["project", "--json"])
        .arg(&server.root)
        .env_remove("LEADLINE_METRICS_DIR")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        body.as_bytes(),
        output.stdout.as_slice(),
        "byte-identical to project --json"
    );

    let (status, content_type, body) =
        exchange(port, "GET /api/telemetry HTTP/1.1\r\nHost: localhost");
    assert_eq!(status, 200);
    assert_eq!(content_type, "application/json");
    let telemetry: Value = serde_json::from_str(&body).unwrap();
    assert!(telemetry.get("series").is_some(), "{body}");

    let (status, content_type, body) = exchange(port, "GET /health HTTP/1.1\r\nHost: localhost");
    assert_eq!(status, 200);
    assert_eq!(content_type, "application/json");
    let health: Value = serde_json::from_str(&body).unwrap();
    assert!(health["analyzer_version"].is_string(), "{body}");
    assert!(health["analyzed_at"].is_number(), "{body}");
    assert!(health["duration_ms"].is_number(), "{body}");
    assert_eq!(health["status"], "ok");
}

#[test]
fn stats_rejects_unknown_paths_and_wrong_methods() {
    let server = spawn_stats_server();
    let port = server.port();

    let (status, content_type, body) = exchange(port, "GET /nope HTTP/1.1\r\nHost: localhost");
    assert_eq!(status, 404);
    assert_eq!(content_type, "application/json");
    let error: Value = serde_json::from_str(&body).unwrap();
    assert!(error.get("error").is_some(), "{body}");

    let (status, content_type, body) =
        exchange(port, "POST /api/project HTTP/1.1\r\nHost: localhost");
    assert_eq!(status, 405);
    assert_eq!(content_type, "application/json");
    let error: Value = serde_json::from_str(&body).unwrap();
    assert!(error.get("error").is_some(), "{body}");
}

/// A successful refresh re-analyzes and reports a `meta.analyzed_at` that is
/// never older than the snapshot `/health` served before it.
#[test]
fn stats_refresh_advances_the_snapshot() {
    let server = spawn_stats_server();
    let port = server.port();
    let (_, _, body) = exchange(port, "GET /health HTTP/1.1\r\nHost: localhost");
    let health: Value = serde_json::from_str(&body).unwrap();
    let before = health["analyzed_at"].as_u64().unwrap();

    let (status, content_type, body) =
        exchange(port, "POST /api/refresh HTTP/1.1\r\nHost: localhost");
    assert_eq!(status, 200);
    assert_eq!(content_type, "application/json");
    let refresh: Value = serde_json::from_str(&body).unwrap();
    let after = refresh["meta"]["analyzed_at"].as_u64().unwrap();
    assert!(after >= before, "{body}");
}

/// A non-loopback browser origin and a rebound `Host` are refused before
/// routing (DNS-rebinding defense).
#[test]
fn stats_rejects_foreign_origins_and_hosts() {
    let server = spawn_stats_server();
    let port = server.port();
    let (status, _, _) = exchange(
        port,
        "GET / HTTP/1.1\r\nHost: localhost\r\nOrigin: http://evil.test",
    );
    assert_eq!(status, 403);
    let (status, _, _) = exchange(port, "GET / HTTP/1.1\r\nHost: evil.test");
    assert_eq!(status, 400);
}

/// The same assets the binary embeds, included here too: a `web/dist` that
/// loses or empties a file fails the build rather than the served page.
#[test]
fn embedded_dashboard_assets_are_complete() {
    let index = include_str!("../web/dist/index.html");
    let app = include_str!("../web/dist/assets/app.js");
    let css = include_str!("../web/dist/assets/index.css");
    assert!(!index.is_empty());
    assert!(index.contains("id=\"root\""));
    assert!(app.len() > 10_000);
    assert!(css.len() > 1_000);
}

/// `web/dist` is rebuilt from `web/src` on every dashboard change: the build
/// writes a FNV-1a fingerprint (`node web/scripts/fingerprint.mjs`) that this
/// test recomputes with no node involved, so a stale bundle fails offline.
#[test]
fn web_dist_matches_web_src_fingerprint() {
    fn walk(dir: &std::path::Path, root: &std::path::Path, found: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, found);
            } else {
                found.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let web = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web");
    let mut files = Vec::new();
    walk(&web.join("src"), &web, &mut files);
    files.sort();
    // FNV-1a 64, matching web/scripts/fingerprint.mjs.
    let mut digest: u64 = 14_695_981_039_346_656_037;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            digest ^= u64::from(*byte);
            digest = digest.wrapping_mul(1_099_511_628_211);
        }
    };
    for file in &files {
        feed(file.as_bytes());
        feed(&[0]);
        feed(&std::fs::read(web.join(file)).unwrap());
    }
    let stored = std::fs::read_to_string(web.join("dist").join(".src-hash")).expect(
        "web/dist/.src-hash is missing: run `npm run build` in web/ to regenerate the bundle",
    );
    assert_eq!(
        format!("{digest:016x}"),
        stored.trim(),
        "web/dist is stale: run `npm run build` in web/ after changing web/src"
    );
}
