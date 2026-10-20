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

/// Current resident memory, not virtual address space or a historical peak.
#[cfg(target_os = "macos")]
fn resident_bytes() -> Option<u64> {
    use std::ffi::{c_int, c_void};

    // sys/proc_info.h: proc_taskinfo, PROC_PIDTASKINFO = 4.
    #[derive(Default)]
    #[repr(C)]
    struct TaskInfo {
        virtual_size: u64,
        resident_size: u64,
        total_user: u64,
        total_system: u64,
        threads_user: u64,
        threads_system: u64,
        counters: [i32; 12],
    }
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut c_void,
            size: c_int,
        ) -> c_int;
    }
    let mut info = TaskInfo::default();
    let size = std::mem::size_of::<TaskInfo>() as c_int;
    // SAFETY: info matches proc_taskinfo's C layout and the supplied buffer size.
    let read = unsafe {
        proc_pidinfo(
            std::process::id() as c_int,
            4,
            0,
            (&mut info as *mut TaskInfo).cast(),
            size,
        )
    };
    (read == size).then_some(info.resident_size)
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
    // SAFETY: the pseudo-handle is valid; counters matches PROCESS_MEMORY_COUNTERS.
    let result = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) };
    (result != 0).then_some(counters.working_set as u64)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn resident_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
