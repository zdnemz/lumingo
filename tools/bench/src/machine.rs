//! What machine a result came from.

use serde::{Deserialize, Serialize};
use sysinfo::System;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineInfo {
    /// The exact string the operating system reports, never a marketing name.
    pub cpu: String,
    pub logical_processors: u32,
    pub ram_mb: u64,
    pub os: String,
}

impl MachineInfo {
    pub fn detect() -> Self {
        let mut system = System::new();
        system.refresh_cpu_all();
        system.refresh_memory();
        let cpu = system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().trim().to_owned())
            .filter(|brand| !brand.is_empty())
            .unwrap_or_else(|| "unknown".to_owned());
        let logical = system.cpus().len();
        let logical = if logical == 0 {
            std::thread::available_parallelism().map_or(1, usize::from)
        } else {
            logical
        };
        Self {
            cpu,
            logical_processors: u32::try_from(logical).unwrap_or(u32::MAX),
            ram_mb: system.total_memory() / (1024 * 1024),
            os: System::long_os_version().unwrap_or_else(|| "unknown".to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MachineInfo;

    #[test]
    fn detection_finds_something_plausible() {
        let machine = MachineInfo::detect();
        assert!(machine.logical_processors >= 1);
        assert!(machine.ram_mb > 0);
        assert!(!machine.cpu.is_empty());
        assert!(!machine.os.is_empty());
    }
}
