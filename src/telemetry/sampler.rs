//! Sampling thread: reads process counters during one invocation.
//!
//! Started only when telemetry is enabled, so the disabled path spawns no
//! thread and performs no counter read. The thread aggregates in memory; only
//! its once-per-second live-gauge flush touches the store.

use super::counters::{self, ProcessSample};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Gap between counter reads.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
/// Live gauges are written at most this often.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// Bucket bounds for sampled CPU utilization, in cores. The store's family
/// table renders the same list, so a divergence would silently drop rows.
pub(crate) const RATIO_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0];

/// One invocation's cumulative distribution over a family's fixed bounds.
///
/// Fields are crate-visible because the parent `telemetry` module merges them
/// into the store.
#[derive(Clone, Debug)]
pub(crate) struct Histogram {
    pub(crate) bounds: &'static [f64],
    pub(crate) counts: Vec<u64>,
    pub(crate) count: u64,
    pub(crate) sum: f64,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            bounds: &[],
            counts: Vec::new(),
            count: 0,
            sum: 0.0,
        }
    }
}

impl Histogram {
    fn for_bounds(bounds: &'static [f64]) -> Self {
        Self {
            bounds,
            counts: vec![0; bounds.len()],
            count: 0,
            sum: 0.0,
        }
    }

    /// Records one sample; counts are cumulative, like Prometheus buckets.
    fn observe(&mut self, value: f64) {
        for (slot, bound) in self.counts.iter_mut().zip(self.bounds.iter()) {
            if value <= *bound {
                *slot = slot.saturating_add(1);
            }
        }
        self.count = self.count.saturating_add(1);
        self.sum += value;
    }

    fn samples(&self) -> u64 {
        self.count
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Cost of one invocation, ready to record.
#[derive(Clone, Debug, Default)]
pub struct ProcessCost {
    /// CPU seconds (user plus system) the invocation consumed.
    pub cpu_seconds: f64,
    /// Peak resident set size in bytes, when the platform reports it.
    pub peak_rss_bytes: Option<u64>,
    cpu_ratio: Histogram,
    rss_bytes: Histogram,
}

impl ProcessCost {
    /// Number of counter samples taken during the invocation.
    pub fn sampled_ticks(&self) -> u64 {
        self.cpu_ratio.samples()
    }

    pub(crate) fn cpu_ratio(&self) -> &Histogram {
        &self.cpu_ratio
    }

    pub(crate) fn rss_histogram(&self) -> &Histogram {
        &self.rss_bytes
    }
}

/// Sampling handle for one invocation.
pub struct Sampler {
    inner: Option<Inner>,
}

struct Inner {
    stop: Arc<(Mutex<LoopState>, Condvar)>,
    handle: Option<std::thread::JoinHandle<Aggregate>>,
    /// When sampling began, for a run shorter than one tick.
    started: Instant,
    /// Process CPU at sampling start, so the reported cost is a delta: the
    /// whole command for the CLI, one tool call for MCP.
    cpu_at_start: Option<f64>,
}

/// State the caller and the sampling thread share: the caller sets `stop` to
/// end sampling, and reads `ticks` to wait for real progress instead of
/// guessing how long the thread needs.
#[derive(Default)]
struct LoopState {
    stop: bool,
    /// CPU-ratio observations recorded so far.
    ticks: u64,
}

#[derive(Default)]
struct Aggregate {
    cpu_ratio: Histogram,
    rss_bytes: Histogram,
    live_cpu_millicores: Option<u64>,
    live_rss_bytes: Option<u64>,
}

impl Aggregate {
    fn new() -> Self {
        Self {
            cpu_ratio: Histogram::for_bounds(RATIO_BUCKETS),
            rss_bytes: Histogram::for_bounds(super::RSS_BUCKETS),
            live_cpu_millicores: None,
            live_rss_bytes: None,
        }
    }
}

impl Sampler {
    /// Starts sampling unless telemetry is disabled.
    pub fn start(surface: &str, operation: &str) -> Sampler {
        if !super::enabled() {
            return Sampler::disabled();
        }
        Sampler::spawn(surface, operation)
    }

    /// Starts sampling regardless of configuration. Callers that need the
    /// real gate use [`Sampler::start`]; this exists for measurement code.
    pub(crate) fn spawn(surface: &str, operation: &str) -> Sampler {
        let started = Instant::now();
        let cpu_at_start = counters::read().map(|sample| sample.cpu_seconds);
        let stop = Arc::new((Mutex::new(LoopState::default()), Condvar::new()));
        let thread_stop = Arc::clone(&stop);
        let surface = surface.to_owned();
        let operation = operation.to_owned();
        let handle = std::thread::Builder::new()
            .name("leadline-sampler".to_owned())
            .spawn(move || sample_loop(&surface, &operation, &thread_stop))
            .ok();
        Sampler {
            inner: Some(Inner {
                stop,
                handle,
                started,
                cpu_at_start,
            }),
        }
    }

    /// The no-op sampler used when telemetry is disabled.
    fn disabled() -> Sampler {
        Sampler { inner: None }
    }

    /// Stops sampling and returns the invocation's cost. The thread wakes
    /// immediately, so a short command pays no join latency.
    pub fn finish(self) -> ProcessCost {
        let Some(inner) = self.inner else {
            return ProcessCost::default();
        };
        {
            let (lock, cvar) = &*inner.stop;
            let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            state.stop = true;
            cvar.notify_all();
        }
        let mut aggregate = inner
            .handle
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default();
        let end = counters::read();
        let elapsed = inner.started.elapsed().as_secs_f64();
        let cpu_seconds = match (inner.cpu_at_start, end) {
            (Some(start), Some(end)) => (end.cpu_seconds - start).max(0.0),
            // Without a start reading the total is the best available figure.
            (None, Some(end)) => end.cpu_seconds,
            _ => 0.0,
        };
        // A run shorter than one tick still carries a single observation.
        if aggregate.cpu_ratio.is_empty() && elapsed > 0.0 && cpu_seconds > 0.0 {
            aggregate.cpu_ratio.observe(cpu_seconds / elapsed);
        }
        if aggregate.rss_bytes.is_empty()
            && let Some(rss) = end.and_then(|sample| sample.rss_bytes)
        {
            aggregate.rss_bytes.observe(rss as f64);
        }
        ProcessCost {
            cpu_seconds,
            peak_rss_bytes: end.and_then(|sample| sample.peak_rss_bytes),
            cpu_ratio: aggregate.cpu_ratio,
            rss_bytes: aggregate.rss_bytes,
        }
    }
}

fn sample_loop(
    surface: &str,
    operation: &str,
    stop: &Arc<(Mutex<LoopState>, Condvar)>,
) -> Aggregate {
    let mut aggregate = Aggregate::new();
    let mut previous: Option<(Instant, ProcessSample)> = None;
    let mut last_flush = Instant::now();
    loop {
        {
            let (lock, cvar) = &**stop;
            let state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let (state, _) = cvar
                .wait_timeout(state, SAMPLE_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.stop {
                break;
            }
        }
        let now = Instant::now();
        let Some(sample) = counters::read() else {
            continue;
        };
        if let Some((previous_at, previous_sample)) = previous {
            let wall = now.duration_since(previous_at).as_secs_f64();
            if wall > 0.0 {
                let ratio = ((sample.cpu_seconds - previous_sample.cpu_seconds) / wall).max(0.0);
                aggregate.cpu_ratio.observe(ratio);
                aggregate.live_cpu_millicores = Some((ratio * 1000.0).round() as u64);
                let (lock, _) = &**stop;
                let mut state = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                state.ticks += 1;
            }
        }
        if let Some(rss) = sample.rss_bytes {
            aggregate.rss_bytes.observe(rss as f64);
            aggregate.live_rss_bytes = Some(rss);
        }
        if last_flush.elapsed() >= FLUSH_INTERVAL {
            super::record_live(
                surface,
                operation,
                aggregate.live_cpu_millicores,
                aggregate.live_rss_bytes,
            );
            last_flush = Instant::now();
        }
        previous = Some((now, sample));
    }
    aggregate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histograms_bucket_against_their_own_bounds() {
        let mut histogram = Histogram::for_bounds(RATIO_BUCKETS);
        for value in [0.02, 0.3, 1.0, 3.0, 40.0] {
            histogram.observe(value);
        }
        assert_eq!(histogram.count, 5);
        assert!((histogram.sum - 44.32).abs() < 1e-9);
        assert_eq!(histogram.counts, vec![1, 1, 1, 2, 2, 3, 3, 4, 4, 4]);
    }

    #[test]
    fn disabled_sampler_reads_nothing() {
        let cost = Sampler::disabled().finish();
        assert_eq!(cost.sampled_ticks(), 0);
        assert_eq!(cost.cpu_seconds, 0.0);
        assert_eq!(cost.peak_rss_bytes, None);
    }

    /// Waits until the sampling thread has recorded `want` CPU-ratio
    /// observations. A loaded runner can delay the thread start and each
    /// 100 ms wait far past any fixed sleep, so the test waits on the progress
    /// the sampler publishes instead of on the clock.
    fn wait_for_ticks(sampler: &Sampler, want: u64) {
        let inner = sampler.inner.as_ref().expect("spawned sampler is live");
        let (lock, _) = &*inner.stop;
        let recorded = || {
            lock.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .ticks
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while recorded() < want {
            assert!(
                Instant::now() < deadline,
                "sampler recorded {} of {want} ticks",
                recorded()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_live_sampler_records_samples() {
        let sampler = Sampler::spawn("cli", "test-sampler");
        wait_for_ticks(&sampler, 2);
        let cost = sampler.finish();
        assert!(cost.sampled_ticks() >= 2, "{cost:?}");
        assert!(cost.peak_rss_bytes.unwrap_or(0) > 0, "{cost:?}");
        assert!(cost.cpu_seconds > 0.0, "{cost:?}");
    }
}
