//! Peak resident memory of this process.

/// Peak resident set size in MB, or `None` where it cannot be read. Linux only,
/// for the same reason as `machine::detect`.
pub fn rss_peak_mb() -> Option<f64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        parse_vm_hwm_mb(&status)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_vm_hwm_mb(status: &str) -> Option<f64> {
    let line = status.lines().find_map(|l| l.strip_prefix("VmHWM:"))?;
    let kb: f64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kb / 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vm_hwm() {
        assert_eq!(parse_vm_hwm_mb("Name: x\nVmHWM:\t    2048 kB\n"), Some(2.0));
        assert_eq!(parse_vm_hwm_mb("Name: x\n"), None);
    }
}
