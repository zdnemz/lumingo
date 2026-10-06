//! Machine detection and the guard that keeps a `floor` tag honest
//! (BENCHMARK_PLAN section 2).

use anyhow::{Context, Result, bail};
use serde::Serialize;

/// The FLOOR profile: at most 4 logical processors and 8.5 GB of physical memory.
const FLOOR_MAX_LOGICAL_PROCESSORS: usize = 4;
const FLOOR_MAX_RAM_MB: u64 = 8704;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Dev,
    Floor,
}

impl std::str::FromStr for Profile {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "dev" => Ok(Self::Dev),
            "floor" => Ok(Self::Floor),
            other => bail!("unknown profile `{other}`, expected `dev` or `floor`"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Machine {
    pub cpu: String,
    pub logical_processors: usize,
    pub ram_mb: u64,
    pub os: String,
}

/// Refuses a `floor` tag on a machine that is not capped.
pub fn check_profile(profile: Profile, machine: &Machine) -> Result<()> {
    if profile == Profile::Floor
        && (machine.logical_processors > FLOOR_MAX_LOGICAL_PROCESSORS
            || machine.ram_mb > FLOOR_MAX_RAM_MB)
    {
        bail!(
            "refusing the `floor` tag: this machine shows {} logical processors and {} MB of RAM, \
             FLOOR allows at most {FLOOR_MAX_LOGICAL_PROCESSORS} and {FLOOR_MAX_RAM_MB} MB",
            machine.logical_processors,
            machine.ram_mb
        );
    }
    Ok(())
}

/// Reads the machine from the OS. Only Linux is implemented: Windows needs either
/// FFI (forbidden by the workspace lints) or a new dependency, which the owner
/// must approve and add to the license register first.
pub fn detect() -> Result<Machine> {
    #[cfg(target_os = "linux")]
    {
        let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").context("reading /proc/cpuinfo")?;
        let meminfo = std::fs::read_to_string("/proc/meminfo").context("reading /proc/meminfo")?;
        let os = std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .context("reading kernel release")?;
        Ok(Machine {
            cpu: parse_cpu_model(&cpuinfo).context("no `model name` line in /proc/cpuinfo")?,
            logical_processors: std::thread::available_parallelism()?.get(),
            ram_mb: parse_mem_total_mb(&meminfo).context("no `MemTotal` line in /proc/meminfo")?,
            os: format!("Linux {}", os.trim()),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        bail!("machine detection is only implemented for Linux so far")
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_cpu_model(cpuinfo: &str) -> Option<String> {
    cpuinfo
        .lines()
        .find_map(|l| l.strip_prefix("model name"))
        .and_then(|rest| rest.split_once(':'))
        .map(|(_, v)| v.trim().to_owned())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_mem_total_mb(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find_map(|l| l.strip_prefix("MemTotal:"))?;
    let kb: u64 = line.trim().strip_suffix("kB")?.trim().parse().ok()?;
    Some(kb / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(logical_processors: usize, ram_mb: u64) -> Machine {
        Machine {
            cpu: "test".into(),
            logical_processors,
            ram_mb,
            os: "test".into(),
        }
    }

    #[test]
    fn floor_is_refused_on_an_uncapped_machine() {
        assert!(check_profile(Profile::Floor, &machine(12, 24576)).is_err());
        assert!(check_profile(Profile::Floor, &machine(4, 24576)).is_err());
        assert!(check_profile(Profile::Floor, &machine(12, 8192)).is_err());
    }

    #[test]
    fn floor_is_accepted_on_a_capped_machine_and_dev_always() {
        assert!(check_profile(Profile::Floor, &machine(4, 8192)).is_ok());
        assert!(check_profile(Profile::Dev, &machine(12, 24576)).is_ok());
    }

    #[test]
    fn parses_proc_files() {
        assert_eq!(
            parse_cpu_model("processor\t: 0\nmodel name\t: Test CPU @ 3GHz\n").as_deref(),
            Some("Test CPU @ 3GHz")
        );
        assert_eq!(
            parse_mem_total_mb("MemTotal:       8388608 kB\nMemFree: 1 kB\n"),
            Some(8192)
        );
    }
}
