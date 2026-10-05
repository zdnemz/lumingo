//! Hardware profile detection.
//!
//! Only what the operating system reports is stored: installed memory and the
//! number of logical processors this process may use. The floor is the minimum
//! hardware of the product requirements (8 GB, 4 logical processors). Nothing
//! here estimates speed; speed is measured by the benchmark harness.

use crate::api::HardwareProfile;

/// 8 GB. Written as a decimal figure on purpose: an "8 GB" machine reports a
/// little less than 8 GiB of usable memory, and counting in GiB would reject it.
pub const MINIMUM_RAM_BYTES: u64 = 8_000_000_000;

/// The product floor for logical processors.
pub const MINIMUM_LOGICAL_CORES: u32 = 4;

/// Builds the profile from measured values. Separate from [`detect`] so the
/// floor comparison can be tested without a machine.
pub fn assess(ram_total_bytes: Option<u64>, logical_cores: Option<u32>) -> HardwareProfile {
    let meets_minimum = match (ram_total_bytes, logical_cores) {
        (Some(ram), Some(cores)) => {
            Some(ram >= MINIMUM_RAM_BYTES && cores >= MINIMUM_LOGICAL_CORES)
        }
        _ => None,
    };
    HardwareProfile {
        ram_total_bytes,
        logical_cores,
        minimum_ram_bytes: MINIMUM_RAM_BYTES,
        minimum_logical_cores: MINIMUM_LOGICAL_CORES,
        meets_minimum,
    }
}

/// Measures this machine. It reads operating-system counters and may touch the
/// disk, so callers on the async runtime use `tokio::task::spawn_blocking`.
pub fn detect() -> HardwareProfile {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    // sysinfo reports 0 when the platform call failed.
    let ram = Some(system.total_memory()).filter(|bytes| *bytes > 0);
    // `available_parallelism` honours processor affinity and, on Windows, job
    // limits, which is the number that matters for a process that is capped
    // to simulate the floor profile.
    let cores = std::thread::available_parallelism()
        .ok()
        .and_then(|n| u32::try_from(n.get()).ok());
    assess(ram, cores)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_is_compared_on_both_values() {
        // (ram, cores, expected)
        let table = [
            (
                Some(MINIMUM_RAM_BYTES),
                Some(MINIMUM_LOGICAL_CORES),
                Some(true),
            ),
            (
                Some(MINIMUM_RAM_BYTES - 1),
                Some(MINIMUM_LOGICAL_CORES),
                Some(false),
            ),
            (
                Some(MINIMUM_RAM_BYTES),
                Some(MINIMUM_LOGICAL_CORES - 1),
                Some(false),
            ),
            (Some(24_000_000_000), Some(12), Some(true)),
            (None, Some(12), None),
            (Some(24_000_000_000), None, None),
            (None, None, None),
        ];
        for (ram, cores, expected) in table {
            assert_eq!(
                assess(ram, cores).meets_minimum,
                expected,
                "{ram:?} {cores:?}"
            );
        }
    }

    #[test]
    fn the_floor_is_eight_decimal_gigabytes_so_a_machine_just_under_eight_gibibytes_passes() {
        let eight_gibibytes = 8 * 1024 * 1024 * 1024;
        assert!(MINIMUM_RAM_BYTES < eight_gibibytes);
        let just_under = eight_gibibytes - 1;
        assert_eq!(assess(Some(just_under), Some(4)).meets_minimum, Some(true));
    }

    #[test]
    fn detect_reports_what_this_machine_has() {
        let profile = detect();
        assert!(profile.logical_cores.is_some_and(|n| n >= 1));
        assert!(profile.ram_total_bytes.is_some_and(|bytes| bytes > 0));
        assert_eq!(profile.minimum_ram_bytes, MINIMUM_RAM_BYTES);
    }
}
