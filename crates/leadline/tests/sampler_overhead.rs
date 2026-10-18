//! Sampler overhead budgets. One measurement test runs all three phases
//! sequentially, so no sibling test competes for this process's CPU:
//! run it with
//! `cargo test --release --offline --locked --test sampler_overhead -- --ignored --nocapture`
//! and point `LEADLINE_OVERHEAD_FIXTURE` at a multi-second analysis input.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
#[ignore = "measurement, not a correctness gate"]
fn sampler_overhead_budgets() {
    counter_read_stays_within_budget();
    idle_sampler_stays_within_budget();
    end_to_end_overhead_stays_within_budget();
}

fn counter_read_stays_within_budget() {
    const CALLS: u32 = 100_000;
    let started = Instant::now();
    for _ in 0..CALLS {
        assert!(leadline::telemetry::process_sample().is_some());
    }
    let per_call = started.elapsed().as_secs_f64() / f64::from(CALLS);
    println!("counters::read(): {:.0} ns/call", per_call * 1e9);
    assert!(per_call < 5e-6, "read budget exceeded: {per_call:.9}s");
}

fn idle_sampler_stays_within_budget() {
    // Let the counter-read loop above settle before measuring this process's
    // CPU again.
    std::thread::sleep(Duration::from_millis(200));
    let before = leadline::telemetry::process_sample()
        .expect("counters")
        .cpu_seconds;
    let sampler = leadline::telemetry::sampler_for_measurement("cli", "idle-budget");
    std::thread::sleep(Duration::from_secs(5));
    let after = leadline::telemetry::process_sample()
        .expect("counters")
        .cpu_seconds;
    let cost = sampler.finish();
    let used = after - before;
    println!(
        "idle sampler: {:.3} ms CPU over 5 s, {} ticks",
        used * 1e3,
        cost.sampled_ticks()
    );
    assert!(used < 0.025, "sampler used {used:.4}s CPU over 5 s");
    assert!(
        cost.sampled_ticks() >= 40,
        "only {} ticks in 5 s",
        cost.sampled_ticks()
    );
}

fn end_to_end_overhead_stays_within_budget() {
    let fixture = std::env::var("LEADLINE_OVERHEAD_FIXTURE")
        .expect("set LEADLINE_OVERHEAD_FIXTURE to a multi-second analysis input");
    let metrics = std::env::temp_dir().join("leadline-overhead-e2e");
    let mut off = Vec::new();
    let mut on = Vec::new();
    for _ in 0..5 {
        off.push(time_analysis(&fixture, None));
        on.push(time_analysis(&fixture, Some(&metrics)));
    }
    off.sort_by(|left, right| left.partial_cmp(right).expect("finite"));
    on.sort_by(|left, right| left.partial_cmp(right).expect("finite"));
    let (median_off, median_on) = (off[2], on[2]);
    let overhead = (median_on - median_off) / median_off;
    println!(
        "end-to-end: off {median_off:.3}s, on {median_on:.3}s, overhead {:+.2}%",
        overhead * 100.0
    );
    assert!(
        overhead < 0.01,
        "overhead {:.2}% above the 1% budget",
        overhead * 100.0
    );
}

/// Runs the built binary once, with or without metrics, and returns seconds.
fn time_analysis(fixture: &str, metrics: Option<&Path>) -> f64 {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_leadline"));
    let started = Instant::now();
    let mut command = Command::new(binary);
    command
        .arg("analyze")
        .arg(fixture)
        .stdout(std::process::Stdio::null());
    match metrics {
        Some(directory) => {
            command.env("LEADLINE_METRICS_DIR", directory);
        }
        None => {
            command.env_remove("LEADLINE_METRICS_DIR");
        }
    }
    let status = command.status().expect("binary runs");
    assert!(status.success(), "{status:?}");
    started.elapsed().as_secs_f64()
}
