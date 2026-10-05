use crate::{
    CancelFlag, EngineInfo, PcmChunk, SttEngine, SttError, Transcript, TtsEngine, TtsError, Vad,
    VadError,
};

/// The engine a build without a speech runtime uses.
///
/// It is not a fake: every call fails with a typed error that says why, so the
/// interface can tell the learner that voice is not installed instead of
/// pretending to listen.
#[derive(Debug, Clone)]
pub struct UnavailableEngine {
    reason: String,
}

impl UnavailableEngine {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    fn info_value(&self) -> EngineInfo {
        EngineInfo::new("unavailable", "none")
    }
}

impl SttEngine for UnavailableEngine {
    fn start(&mut self) -> Result<(), SttError> {
        Err(SttError::Unavailable {
            reason: self.reason.clone(),
        })
    }
    fn feed(&mut self, _samples: &[f32]) -> Result<(), SttError> {
        Err(SttError::Unavailable {
            reason: self.reason.clone(),
        })
    }
    fn finish(&mut self, _cancel: &CancelFlag) -> Result<Transcript, SttError> {
        Err(SttError::Unavailable {
            reason: self.reason.clone(),
        })
    }
    fn info(&self) -> EngineInfo {
        self.info_value()
    }
}

impl TtsEngine for UnavailableEngine {
    fn synthesize(&mut self, _text: &str, _cancel: &CancelFlag) -> Result<PcmChunk, TtsError> {
        Err(TtsError::Unavailable {
            reason: self.reason.clone(),
        })
    }
    fn info(&self) -> EngineInfo {
        self.info_value()
    }
}

impl Vad for UnavailableEngine {
    fn push_frame(&mut self, _frame: &[f32]) -> Result<f32, VadError> {
        Err(VadError::Unavailable {
            reason: self.reason.clone(),
        })
    }
    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_call_fails_with_the_reason() {
        let mut engine = UnavailableEngine::new("built without the sherpa feature");
        let cancel = CancelFlag::new();
        assert!(matches!(
            SttEngine::start(&mut engine),
            Err(SttError::Unavailable { reason }) if reason.contains("sherpa")
        ));
        assert!(matches!(
            engine.synthesize("hi", &cancel),
            Err(TtsError::Unavailable { .. })
        ));
        assert!(matches!(
            engine.push_frame(&[0.0; 512]),
            Err(VadError::Unavailable { .. })
        ));
        assert_eq!(SttEngine::info(&engine).id, "unavailable");
    }
}
