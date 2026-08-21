use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct PeakRssSample {
    pub bytes: Option<u64>,
    pub provider: &'static str,
    pub error: Option<String>,
}

impl PeakRssSample {
    pub fn to_json(&self) -> Value {
        json!({
            "bytes": self.bytes,
            "provider": self.provider,
            "scope": "process high-water mark since process start; values after individual cases are cumulative, not per-case deltas",
            "error": self.error,
        })
    }
}

#[cfg(target_os = "linux")]
pub fn sample() -> PeakRssSample {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `getrusage` initializes the complete `rusage` structure when it
    // returns zero, and the pointer is valid for the duration of the call.
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if status != 0 {
        return PeakRssSample {
            bytes: None,
            provider: "getrusage(RUSAGE_SELF).ru_maxrss (KiB on Linux)",
            error: Some(std::io::Error::last_os_error().to_string()),
        };
    }
    // SAFETY: successful `getrusage` initialized the structure above.
    let usage = unsafe { usage.assume_init() };
    PeakRssSample {
        bytes: u64::try_from(usage.ru_maxrss)
            .ok()
            .and_then(|kibibytes| kibibytes.checked_mul(1_024)),
        provider: "getrusage(RUSAGE_SELF).ru_maxrss (KiB on Linux)",
        error: None,
    }
}

#[cfg(target_os = "macos")]
pub fn sample() -> PeakRssSample {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: `getrusage` initializes the complete `rusage` structure when it
    // returns zero, and the pointer is valid for the duration of the call.
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if status != 0 {
        return PeakRssSample {
            bytes: None,
            provider: "getrusage(RUSAGE_SELF).ru_maxrss (bytes on macOS)",
            error: Some(std::io::Error::last_os_error().to_string()),
        };
    }
    // SAFETY: successful `getrusage` initialized the structure above.
    let usage = unsafe { usage.assume_init() };
    PeakRssSample {
        bytes: u64::try_from(usage.ru_maxrss).ok(),
        provider: "getrusage(RUSAGE_SELF).ru_maxrss (bytes on macOS)",
        error: None,
    }
}

#[cfg(windows)]
pub fn sample() -> PeakRssSample {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = std::mem::MaybeUninit::<PROCESS_MEMORY_COUNTERS>::zeroed();
    // SAFETY: the pseudo-handle returned by `GetCurrentProcess` is always valid
    // in the current process, and the writable buffer has the advertised size.
    let status = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            counters.as_mut_ptr(),
            u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(u32::MAX),
        )
    };
    if status == 0 {
        return PeakRssSample {
            bytes: None,
            provider: "GetProcessMemoryInfo.PeakWorkingSetSize",
            error: Some(std::io::Error::last_os_error().to_string()),
        };
    }
    // SAFETY: successful `GetProcessMemoryInfo` initialized the structure.
    let counters = unsafe { counters.assume_init() };
    PeakRssSample {
        bytes: Some(counters.PeakWorkingSetSize as u64),
        provider: "GetProcessMemoryInfo.PeakWorkingSetSize",
        error: None,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn sample() -> PeakRssSample {
    PeakRssSample {
        bytes: None,
        provider: "unsupported platform",
        error: Some(
            "peak RSS collection is implemented only for Windows, Linux, and macOS".to_owned(),
        ),
    }
}
