//! Per-platform process counters: CPU time and resident memory.
//!
//! Every platform funnels into [`read`]. A platform that cannot answer, or a
//! call that fails, returns `None`, and the caller records nothing rather than
//! fabricating a value. Nothing here may panic, block, or write.

/// One process-wide reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessSample {
    /// Cumulative processor time (user plus system) in seconds.
    pub cpu_seconds: f64,
    /// Current resident set size in bytes.
    pub rss_bytes: Option<u64>,
    /// Peak resident set size in bytes.
    pub peak_rss_bytes: Option<u64>,
}

#[cfg(unix)]
pub(crate) fn read() -> Option<ProcessSample> {
    // SAFETY: `getrusage` writes a fully initialized `rusage` for RUSAGE_SELF
    // and reads nothing else; the pointer is valid for the call's duration.
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    // SAFETY: `getrusage` returned 0, so the structure was initialized.
    let usage = unsafe { usage.assume_init() };
    let cpu_seconds = usage.ru_utime.tv_sec as f64
        + usage.ru_utime.tv_usec as f64 / 1_000_000.0
        + usage.ru_stime.tv_sec as f64
        + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
    Some(ProcessSample {
        cpu_seconds,
        rss_bytes: current_rss_bytes(),
        peak_rss_bytes: peak_rss_bytes(&usage),
    })
}

/// macOS reports `ru_maxrss` in bytes; the other Unixes report KiB.
#[cfg(unix)]
fn peak_rss_bytes(usage: &libc::rusage) -> Option<u64> {
    let raw = u64::try_from(usage.ru_maxrss).ok()?;
    if cfg!(target_os = "macos") {
        Some(raw)
    } else {
        raw.checked_mul(1024)
    }
}

/// macOS current RSS from `proc_pid_rusage`, which the kernel answers for the
/// calling process without extra privileges.
#[cfg(target_os = "macos")]
fn current_rss_bytes() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: `proc_pid_rusage` fills a `rusage_info_v2` for our own pid when
    // the flavor matches the buffer type; the pointer is valid for the call.
    let result = unsafe {
        libc::proc_pid_rusage(
            libc::getpid(),
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast::<*mut libc::c_void>(),
        )
    };
    if result != 0 {
        return None;
    }
    // SAFETY: a zero return means the structure was filled.
    let info = unsafe { info.assume_init() };
    Some(info.ri_resident_size)
}

/// Linux current RSS is the resident field of `/proc/self/statm`, in pages.
#[cfg(all(unix, not(target_os = "macos")))]
fn current_rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: `sysconf` only reads a process-wide constant.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    pages.checked_mul(u64::try_from(page_size).ok()?)
}

#[cfg(windows)]
pub(crate) fn read() -> Option<ProcessSample> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    // SAFETY: the current-process pseudo-handle needs no cleanup.
    let handle = unsafe { GetCurrentProcess() };
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: the handle is the current process, and `cb` names the struct size.
    let sized = unsafe { GetProcessMemoryInfo(handle, &mut counters, counters.cb) };
    if sized == 0 {
        return None;
    }

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: every pointer names a distinct, correctly typed, writable slot.
    let times =
        unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
    if times == 0 {
        return None;
    }
    let cpu_seconds = (filetime_ticks(kernel) + filetime_ticks(user)) as f64 / 10_000_000.0;
    Some(ProcessSample {
        cpu_seconds,
        rss_bytes: Some(counters.WorkingSetSize as u64),
        peak_rss_bytes: Some(counters.PeakWorkingSetSize as u64),
    })
}

/// `FILETIME` counts 100 ns intervals across two 32-bit halves.
#[cfg(windows)]
fn filetime_ticks(time: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

/// Every other target records no cost series instead of guessing.
#[cfg(not(any(unix, windows)))]
pub(crate) fn read() -> Option<ProcessSample> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(unix, windows))]
    fn reads_plausible_process_counters() {
        let first = read().expect("this platform reports process counters");
        let mut spin = 0u64;
        for index in 0..2_000_000u64 {
            spin = spin.wrapping_add(index % 7);
        }
        assert!(spin > 0);
        let second = read().expect("counters stay readable");
        assert!(
            second.cpu_seconds >= first.cpu_seconds,
            "{first:?} {second:?}"
        );
        let rss = second.rss_bytes.expect("resident size is reported");
        assert!(rss > 0, "{second:?}");
        let peak = second
            .peak_rss_bytes
            .expect("peak resident size is reported");
        assert!(peak >= rss, "peak {peak} below current {rss}");
    }
}
