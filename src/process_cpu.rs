use qtbridge::qobject;
use std::time::{Duration, Instant};

/// One logical core at full utilization is 100%; child processes are excluded.
#[derive(Default)]
struct Sampler {
    previous: Option<(Instant, Duration)>,
}

impl Sampler {
    fn sample(&mut self, now: Instant, cpu: Option<Duration>) -> Option<f64> {
        let Some(cpu) = cpu else {
            self.previous = None;
            return None;
        };
        let (previous_time, previous_cpu) = self.previous.replace((now, cpu))?;
        let elapsed = now.checked_duration_since(previous_time)?;
        let used = cpu.checked_sub(previous_cpu)?;
        if elapsed.is_zero() {
            return None;
        }
        Some(100.0 * used.as_secs_f64() / elapsed.as_secs_f64())
    }
}

pub struct ProcessCpu {
    sampler: Sampler,
    logical_processors: Option<usize>,
    usage: String,
}

impl Default for ProcessCpu {
    fn default() -> Self {
        let mut sampler = Sampler::default();
        sampler.sample(Instant::now(), process_time());
        Self {
            sampler,
            logical_processors: cpu_divisor(),
            usage: "CPU —".into(),
        }
    }
}

#[qobject]
impl ProcessCpu {
    qproperty!("usage", Member = usage, Notify = changed);

    #[qsignal]
    fn changed(&mut self);

    #[qslot]
    fn poll(&mut self) {
        let cpu = process_time();
        let usage = self
            .sampler
            .sample(Instant::now(), cpu)
            .and_then(|value| normalize_usage(value, self.logical_processors));
        let text = usage.map_or_else(|| "CPU —".into(), |value| format!("CPU {value:.1}%"));
        if self.usage != text {
            self.usage = text;
            self.changed();
        }
    }
}

// Windows Task Manager uses total machine capacity; Unix tools conventionally
// report one fully utilized logical processor as 100%.
fn normalize_usage(value: f64, logical_processors: Option<usize>) -> Option<f64> {
    logical_processors
        .filter(|&count| count > 0)
        .map(|count| value / count as f64)
}

#[cfg(target_os = "windows")]
fn cpu_divisor() -> Option<usize> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetActiveProcessorCount(group_number: u16) -> u32;
    }
    // ALL_PROCESSOR_GROUPS: include machines with more than 64 logical CPUs,
    // rather than limiting normalization to the calling thread's group/affinity.
    // SAFETY: this API accepts the documented all-groups constant, no pointers.
    let count = unsafe { GetActiveProcessorCount(0xffff) } as usize;
    (count > 0).then_some(count)
}

#[cfg(not(target_os = "windows"))]
fn cpu_divisor() -> Option<usize> {
    Some(1)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn process_time() -> Option<Duration> {
    use std::ffi::{c_int, c_long};

    #[repr(C)]
    struct Timespec {
        seconds: c_long,
        nanoseconds: c_long,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: c_int, time: *mut Timespec) -> c_int;
    }
    #[cfg(target_os = "macos")]
    const PROCESS_CLOCK: c_int = 12;
    #[cfg(target_os = "linux")]
    const PROCESS_CLOCK: c_int = 2;

    let mut time = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    // SAFETY: time is a writable timespec with the platform's C ABI layout.
    let result = unsafe { clock_gettime(PROCESS_CLOCK, &mut time) };
    if result != 0 || time.seconds < 0 || !(0..1_000_000_000).contains(&time.nanoseconds) {
        return None;
    }
    Some(Duration::new(time.seconds as u64, time.nanoseconds as u32))
}

#[cfg(target_os = "windows")]
fn process_time() -> Option<Duration> {
    use std::ffi::c_void;

    #[derive(Default)]
    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    impl FileTime {
        fn ticks(&self) -> u64 {
            (u64::from(self.high) << 32) | u64::from(self.low)
        }
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn GetProcessTimes(
            process: *mut c_void,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    // SAFETY: the pseudo-handle is valid and all four output pointers are writable.
    let result = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if result == 0 {
        return None;
    }
    let ticks = kernel.ticks().checked_add(user.ticks())?;
    Some(Duration::new(
        ticks / 10_000_000,
        ((ticks % 10_000_000) * 100) as u32,
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn process_time() -> Option<Duration> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_machine_usage_without_changing_single_core_scale() {
        assert_eq!(normalize_usage(32.0, Some(10)), Some(3.2));
        assert_eq!(normalize_usage(1000.0, Some(10)), Some(100.0));
        assert_eq!(normalize_usage(200.0, Some(1)), Some(200.0));
        assert_eq!(normalize_usage(0.0, Some(10)), Some(0.0));
        assert_eq!(normalize_usage(32.0, Some(0)), None);
        assert_eq!(normalize_usage(32.0, None), None);
        assert!(cpu_divisor().is_some_and(|count| count > 0));
    }

    #[test]
    fn measures_interval_usage_including_multiple_cores() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        assert_eq!(sampler.sample(now, Some(Duration::from_secs(10))), None);
        assert_eq!(
            sampler.sample(
                now + Duration::from_secs(2),
                Some(Duration::from_millis(10500))
            ),
            Some(25.0)
        );
        assert_eq!(
            sampler.sample(
                now + Duration::from_secs(3),
                Some(Duration::from_millis(12500))
            ),
            Some(200.0)
        );
        assert_eq!(
            sampler.sample(
                now + Duration::from_secs(4),
                Some(Duration::from_millis(12500))
            ),
            Some(0.0)
        );
    }

    #[test]
    fn recovers_from_missing_or_invalid_samples() {
        let now = Instant::now();
        let mut sampler = Sampler::default();
        sampler.sample(now, Some(Duration::from_secs(2)));
        assert_eq!(sampler.sample(now, Some(Duration::from_secs(2))), None);
        assert_eq!(
            sampler.sample(now + Duration::from_secs(1), Some(Duration::from_secs(1))),
            None
        );
        assert_eq!(sampler.sample(now + Duration::from_secs(2), None), None);
        assert_eq!(
            sampler.sample(now + Duration::from_secs(3), Some(Duration::from_secs(3))),
            None
        );
        assert_eq!(
            sampler.sample(now + Duration::from_secs(4), Some(Duration::from_secs(4))),
            Some(100.0)
        );
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn platform_process_clock_is_monotonic() {
        let before = process_time().expect("process CPU clock");
        let after = process_time().expect("process CPU clock");
        assert!(after >= before);
    }
}
