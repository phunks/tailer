use qtbridge::qobject;

fn format_usage(bytes: Option<u64>) -> String {
    bytes.map_or_else(
        || "RAM —".into(),
        |bytes| format!("RAM {:.1} MiB", bytes as f64 / (1024.0 * 1024.0)),
    )
}

pub struct ProcessMemory {
    usage: String,
}

impl Default for ProcessMemory {
    fn default() -> Self {
        Self {
            usage: format_usage(resident_bytes()),
        }
    }
}

#[qobject]
impl ProcessMemory {
    qproperty!("usage", Member = usage, Notify = changed);

    #[qsignal]
    fn changed(&mut self);

    #[qslot]
    fn poll(&mut self) {
        let usage = format_usage(resident_bytes());
        if self.usage != usage {
            self.usage = usage;
            self.changed();
        }
    }
}

/// Current physical footprint, comparable to Activity Monitor's Memory column.
#[cfg(target_os = "macos")]
fn resident_bytes() -> Option<u64> {
    use std::ffi::{c_int, c_void};

    // sys/resource.h: rusage_info_v0, RUSAGE_INFO_V0 = 0.
    #[derive(Default)]
    #[repr(C)]
    struct UsageInfo {
        uuid: [u8; 16],
        user_time: u64,
        system_time: u64,
        package_idle_wakeups: u64,
        interrupt_wakeups: u64,
        pageins: u64,
        wired_size: u64,
        resident_size: u64,
        physical_footprint: u64,
        process_start: u64,
        process_exit: u64,
    }
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
    }
    let mut info = UsageInfo::default();
    // The API's rusage_info_t* is an opaque output buffer, not a pointer to an
    // allocated pointer. V0's layout is 16 UUID bytes followed by ten u64s.
    // SAFETY: info is writable and matches the selected rusage_info_v0 layout.
    let result = unsafe {
        proc_pid_rusage(
            std::process::id() as c_int,
            0,
            (&mut info as *mut UsageInfo).cast(),
        )
    };
    (result == 0).then_some(info.physical_footprint)
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_status(status: &str) -> Option<u64> {
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    let mut fields = value.split_whitespace();
    let kib = fields.next()?.parse::<u64>().ok()?;
    if fields.next()? != "kB" || fields.next().is_some() {
        return None;
    }
    kib.checked_mul(1024)
}

#[cfg(target_os = "linux")]
fn resident_bytes() -> Option<u64> {
    parse_linux_status(&std::fs::read_to_string("/proc/self/status").ok()?)
}

#[cfg(target_os = "windows")]
fn resident_bytes() -> Option<u64> {
    use std::ffi::c_void;

    if !supports_private_working_set() {
        return None;
    }
    #[derive(Default)]
    #[repr(C)]
    struct MemoryCounters {
        size: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged_pool: usize,
        quota_paged_pool: usize,
        quota_peak_non_paged_pool: usize,
        quota_non_paged_pool: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private_usage: usize,
        private_working_set: usize,
        shared_commit_usage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut MemoryCounters,
            size: u32,
        ) -> i32;
    }
    let size = std::mem::size_of::<MemoryCounters>() as u32;
    let mut counters = MemoryCounters {
        size,
        ..Default::default()
    };
    // PROCESS_MEMORY_COUNTERS_EX2 (Windows 10 1703+). Do not substitute
    // PrivateUsage: that is committed private memory, not resident private pages.
    // SAFETY: the pseudo-handle is valid; counters matches the EX2 C layout.
    let result = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (result != 0).then_some(counters.private_working_set as u64)
}

#[cfg(target_os = "windows")]
fn supports_private_working_set() -> bool {
    use std::sync::OnceLock;

    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        #[repr(C)]
        struct VersionInfo {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform: u32,
            service_pack: [u16; 128],
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn RtlGetVersion(version: *mut VersionInfo) -> i32;
        }
        let mut version = VersionInfo {
            size: std::mem::size_of::<VersionInfo>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            service_pack: [0; 128],
        };
        // RtlGetVersion is not affected by the application's compatibility manifest.
        // SAFETY: version is a writable OSVERSIONINFOW with its correct size.
        let result = unsafe { RtlGetVersion(&mut version) };
        result == 0 && private_working_set_supported(version.major, version.build)
    })
}

#[cfg(any(target_os = "windows", test))]
fn private_working_set_supported(major: u32, build: u32) -> bool {
    major > 10 || (major == 10 && build >= 15063)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn resident_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_working_set_requires_windows_10_1703_or_later() {
        assert!(!private_working_set_supported(6, 9600));
        assert!(!private_working_set_supported(10, 14393));
        assert!(!private_working_set_supported(10, 15062));
        assert!(private_working_set_supported(10, 15063));
        assert!(private_working_set_supported(10, 26100));
        assert!(private_working_set_supported(11, 1));
    }

    #[test]
    fn formats_current_memory_in_mib_or_unavailable() {
        assert_eq!(format_usage(None), "RAM —");
        assert_eq!(format_usage(Some(0)), "RAM 0.0 MiB");
        assert_eq!(
            format_usage(Some(128 * 1024 * 1024 + 512 * 1024)),
            "RAM 128.5 MiB"
        );
        assert_eq!(format_usage(Some(2 * 1024 * 1024 * 1024)), "RAM 2048.0 MiB");
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn platform_reports_current_resident_memory() {
        #[cfg(target_os = "windows")]
        if !supports_private_working_set() {
            assert_eq!(resident_bytes(), None);
            assert_eq!(ProcessMemory::default().usage, "RAM —");
            return;
        }
        assert!(resident_bytes().expect("current resident memory") > 0);
        assert!(ProcessMemory::default().usage.ends_with(" MiB"));
    }

    #[test]
    fn parses_rss_without_using_peak_or_virtual_memory() {
        assert_eq!(
            parse_linux_status("VmPeak:\t9000 kB\nVmSize:\t8000 kB\nVmRSS:\t1234 kB\n"),
            Some(1234 * 1024)
        );
        for status in [
            "",
            "VmRSS: invalid kB",
            "VmRSS: 1 MB",
            "VmRSS: 1",
            "VmRSS: 1 kB extra",
            "VmRSS: 18446744073709551615 kB",
        ] {
            assert_eq!(parse_linux_status(status), None);
        }
    }
}
