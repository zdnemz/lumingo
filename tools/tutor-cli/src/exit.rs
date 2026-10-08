//! Exit codes. Each failure a script may want to tell apart has its own.

use std::process::ExitCode;

/// How the program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The conversation or the script run finished.
    Done,
    /// Anything else that went wrong: a bad file, a bad option, an internal error.
    Failure,
    /// The provider could not be reached or kept failing. The session was stopped
    /// cleanly and the result file, if any, was written.
    ProviderUnavailable,
    /// A speech engine or an audio device is not available or failed to load.
    SpeechUnavailable,
    /// The learner pressed Ctrl-C. Everything was stopped cleanly.
    Interrupted,
}

impl Exit {
    pub fn code(self) -> u8 {
        match self {
            Self::Done => 0,
            Self::Failure => 1,
            // 2 is what clap uses for a usage error.
            Self::ProviderUnavailable => 3,
            Self::SpeechUnavailable => 4,
            Self::Interrupted => 130,
        }
    }
}

impl From<Exit> for ExitCode {
    fn from(exit: Exit) -> Self {
        ExitCode::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exit_has_its_own_code_and_none_is_the_usage_code() {
        let all = [
            Exit::Done,
            Exit::Failure,
            Exit::ProviderUnavailable,
            Exit::SpeechUnavailable,
            Exit::Interrupted,
        ];
        let mut codes: Vec<u8> = all.iter().map(|e| e.code()).collect();
        assert!(!codes.contains(&2));
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), all.len());
    }
}
