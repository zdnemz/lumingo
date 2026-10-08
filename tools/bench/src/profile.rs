//! Hardware profiles and the guard that keeps a `floor` tag honest.

use serde::{Deserialize, Serialize};

use crate::machine::MachineInfo;

/// The largest memory a machine may report and still count as the 8 GB floor.
/// 8.5 GB in MiB, which leaves room for what the operating system reserves.
pub const FLOOR_MAX_RAM_MB: u64 = 8704;
pub const FLOOR_MAX_LOGICAL_PROCESSORS: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    Dev,
    Floor,
}

impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Floor => "floor",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    #[error(
        "refusing the floor profile: this machine reports {logical_processors} logical processors \
         (at most {FLOOR_MAX_LOGICAL_PROCESSORS} allowed) and {ram_mb} MB of memory \
         (at most {FLOOR_MAX_RAM_MB} MB allowed). Cap the machine first, or run with --profile dev"
    )]
    NotCapped {
        logical_processors: u32,
        ram_mb: u64,
    },
}

/// A run may be tagged `floor` only on a machine that is actually capped to it.
/// The `dev` tag is always allowed.
pub fn check_profile(profile: Profile, machine: &MachineInfo) -> Result<(), GuardError> {
    if profile == Profile::Dev {
        return Ok(());
    }
    if machine.logical_processors <= FLOOR_MAX_LOGICAL_PROCESSORS
        && machine.ram_mb <= FLOOR_MAX_RAM_MB
    {
        Ok(())
    } else {
        Err(GuardError::NotCapped {
            logical_processors: machine.logical_processors,
            ram_mb: machine.ram_mb,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(cpus: u32, ram_mb: u64) -> MachineInfo {
        MachineInfo {
            cpu: "test cpu".into(),
            logical_processors: cpus,
            ram_mb,
            os: "test os".into(),
        }
    }

    #[test]
    fn dev_is_always_allowed() {
        assert!(check_profile(Profile::Dev, &machine(32, 65_536)).is_ok());
    }

    #[test]
    fn floor_needs_both_caps() {
        assert!(check_profile(Profile::Floor, &machine(4, 8192)).is_ok());
        assert!(check_profile(Profile::Floor, &machine(2, 4096)).is_ok());
        assert!(check_profile(Profile::Floor, &machine(4, FLOOR_MAX_RAM_MB)).is_ok());
    }

    #[test]
    fn floor_is_refused_when_either_cap_is_exceeded() {
        assert!(check_profile(Profile::Floor, &machine(12, 8192)).is_err());
        assert!(check_profile(Profile::Floor, &machine(4, 24_576)).is_err());
        assert!(check_profile(Profile::Floor, &machine(5, FLOOR_MAX_RAM_MB + 1)).is_err());
        assert!(check_profile(Profile::Floor, &machine(4, FLOOR_MAX_RAM_MB + 1)).is_err());
    }

    #[test]
    fn the_refusal_says_what_was_seen() {
        let error = check_profile(Profile::Floor, &machine(12, 24_576)).expect_err("refused");
        let text = error.to_string();
        assert!(text.contains("12 logical processors"), "{text}");
        assert!(text.contains("24576 MB"), "{text}");
    }
}
