//! Settings of the voice loop and the scenario it plays.

use std::time::Duration;

use assessment_engine::Level;
use curriculum::{Activity, RoleplayMode, Unit};
use speech::{EndpointConfig, SttWorkerConfig, TtsWorkerConfig};
use tutor_engine::{Channel, FeedbackMode, Focus, ObjectiveRef, TutorContext};

use super::error::{VoiceError, VoiceResult};

/// The most the capture side may fall behind before frames are dropped, in
/// frames. 256 frames of 512 samples are 8.2 s of audio.
pub const DEFAULT_FRAME_QUEUE: usize = 256;

/// Every setting has a default taken from `context_pack.md` sections 4 and 13.
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    /// Samples per frame handed to the VAD, at 16 kHz. Silero wants 512.
    pub frame_len: usize,
    pub endpoint: EndpointConfig,
    /// Capacity of the queue from the capture worker to the listener thread, in
    /// frames. When it is full the new frame is dropped and counted
    /// (`VoiceStats::frames_dropped`); the capture worker never waits.
    pub frame_queue: usize,
    /// Capacity of the inbox every producer writes to. Threads use `try_send`
    /// and drop and count on overflow (`VoiceStats::inbox_dropped`); async
    /// callers wait for room.
    pub inbox: usize,
    /// Capacity of the event broadcast. A receiver that falls behind loses the
    /// oldest events and is told how many; the loop never waits for it.
    pub event_capacity: usize,
    /// Utterances that may wait while a turn is running. Further ones are dropped
    /// and counted (`VoiceStats::utterances_dropped`).
    pub utterance_backlog: usize,
    /// How long the loop waits for a transcript (STT limit, section 13).
    pub stt_timeout: Duration,
    /// How long one sentence may take from the TTS worker (section 13).
    pub tts_timeout: Duration,
    /// The whole time budget for the provider to start a reply, both attempts
    /// together. Each attempt gets half. Section 13 gives connect 5 s and first
    /// token 8 s, so 16 s leaves an attempt its full first-token limit.
    pub provider_timeout: Duration,
    /// How long the loop waits for models to load (section 13).
    pub model_load_timeout: Duration,
    /// How often the loop looks at the output counters while it waits for the
    /// first tutor sample, and for playback to drain.
    pub output_poll: Duration,
    /// How often a sentence that the TTS queue refused is offered again.
    pub tts_retry: Duration,
    /// Extra time allowed for playback to drain after the queued audio's own
    /// length, before the loop gives up and reports a playback fault.
    pub drain_grace: Duration,
    /// Capacity of the queue to the turn recorder.
    pub recorder_queue: usize,
    pub stt: SttWorkerConfig,
    pub tts: TtsWorkerConfig,
    /// Words the tutor-first opening turn says to the model.
    pub opening_instruction: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            frame_len: 512,
            endpoint: EndpointConfig::new(),
            frame_queue: DEFAULT_FRAME_QUEUE,
            inbox: 64,
            event_capacity: 64,
            utterance_backlog: 4,
            stt_timeout: Duration::from_secs(10),
            tts_timeout: Duration::from_secs(5),
            provider_timeout: Duration::from_secs(16),
            model_load_timeout: Duration::from_secs(60),
            output_poll: Duration::from_millis(2),
            tts_retry: Duration::from_millis(5),
            drain_grace: Duration::from_secs(2),
            recorder_queue: 16,
            stt: SttWorkerConfig::default(),
            tts: TtsWorkerConfig::default(),
            opening_instruction:
                "Begin the conversation now: greet the learner in your role and ask your first question."
                    .to_owned(),
        }
    }
}

impl VoiceConfig {
    pub(crate) fn validate(&self) -> VoiceResult<()> {
        self.endpoint.validate()?;
        if self.frame_len != self.endpoint.frame_samples {
            return Err(VoiceError::Settings(
                "the frame length must equal the endpointer's frame size",
            ));
        }
        if self.frame_queue == 0 || self.inbox == 0 || self.event_capacity == 0 {
            return Err(VoiceError::Settings(
                "a queue capacity of zero is not usable",
            ));
        }
        if self.recorder_queue == 0 || self.utterance_backlog == 0 {
            return Err(VoiceError::Settings(
                "a queue capacity of zero is not usable",
            ));
        }
        if self.provider_timeout < Duration::from_millis(2) {
            return Err(VoiceError::Settings("the provider timeout is too short"));
        }
        if self.output_poll.is_zero() || self.tts_retry.is_zero() {
            return Err(VoiceError::Settings(
                "a polling interval of zero is not usable",
            ));
        }
        Ok(())
    }
}

/// What the tutor plays: the prompt context and what the analysis is given.
#[derive(Debug, Clone)]
pub struct Scenario {
    /// The context of the T1 prompt, on the voice channel.
    pub context: TutorContext,
    pub unit_id: Option<String>,
    pub activity_id: Option<String>,
    /// For the background analysis: what the turn may give evidence for.
    pub objectives: Vec<ObjectiveRef>,
    pub target_language: Vec<String>,
    /// The roleplay's own turn limit, when it has one. The loop does not stop
    /// by itself; the program that runs it decides.
    pub max_turns: Option<u8>,
}

fn level_of(level: curriculum::Level) -> Level {
    match level {
        curriculum::Level::A1 => Level::A1,
        curriculum::Level::A2 => Level::A2,
        curriculum::Level::B1 => Level::B1,
        curriculum::Level::B2 => Level::B2,
        curriculum::Level::C1 => Level::C1,
        curriculum::Level::C2 => Level::C2,
    }
}

impl Scenario {
    /// The scenario of one roleplay activity of `unit`: the activity named
    /// `activity_id`, or the first roleplay when none is named. `mode` replaces
    /// the activity's own feedback mode. `first_language` is the learner's first
    /// language written in English.
    pub fn from_unit(
        unit: &Unit,
        activity_id: Option<&str>,
        mode: Option<FeedbackMode>,
        first_language: &str,
    ) -> VoiceResult<Self> {
        let roleplay = unit
            .activities
            .iter()
            .find_map(|activity| match activity {
                Activity::Roleplay(r) if activity_id.is_none_or(|id| r.id == id) => Some(r),
                _ => None,
            })
            .ok_or_else(|| VoiceError::NoScenario(activity_id.map(str::to_owned)))?;

        let mut target_language: Vec<String> = unit
            .targets
            .vocabulary
            .iter()
            .filter(|v| roleplay.target_vocab_ids.contains(&v.id))
            .map(|v| v.lemma.clone())
            .collect();
        target_language.extend(
            unit.targets
                .grammar
                .iter()
                .filter(|g| roleplay.target_grammar_ids.contains(&g.id))
                .map(|g| g.pattern.clone()),
        );
        let objectives = unit
            .objectives
            .iter()
            .filter(|o| roleplay.objective_ids.contains(&o.id))
            .map(|o| ObjectiveRef {
                id: o.id.clone(),
                can_do: o.can_do.en.clone(),
            })
            .collect();
        let own_mode = match roleplay.mode {
            RoleplayMode::Fluency => FeedbackMode::Fluency,
            RoleplayMode::Accuracy => FeedbackMode::Accuracy,
        };
        Ok(Self {
            context: TutorContext {
                channel: Channel::Voice,
                level: level_of(unit.level),
                first_language: first_language.to_owned(),
                focus: Focus::Unit(unit.title.en.clone()),
                scenario: roleplay.scenario.en.clone(),
                tutor_role: roleplay.tutor_role.clone(),
                learner_role: roleplay.learner_role.clone(),
                goals: roleplay.goals.clone(),
                target_language: target_language.clone(),
                mode: mode.unwrap_or(own_mode),
                pronunciation_findings: false,
            },
            unit_id: Some(unit.id.clone()),
            activity_id: Some(roleplay.id.clone()),
            objectives,
            target_language,
            max_turns: Some(roleplay.max_turns),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_unit() -> Unit {
        let text = include_str!("../../../../curriculum/examples/a1-u01.example.json");
        curriculum::load_unit_bytes(text.as_bytes())
            .expect("the example unit loads")
            .unit
    }

    #[test]
    fn the_example_unit_gives_the_classmate_roleplay() {
        let unit = example_unit();
        let scenario = Scenario::from_unit(&unit, None, None, "Indonesian").expect("scenario");
        assert_eq!(
            scenario.activity_id.as_deref(),
            Some("a11-roleplay-classmate")
        );
        assert_eq!(scenario.unit_id.as_deref(), Some("a1-u01"));
        let ctx = &scenario.context;
        assert_eq!(ctx.channel, Channel::Voice);
        assert_eq!(ctx.level, Level::A1);
        assert_eq!(ctx.mode, FeedbackMode::Fluency);
        assert_eq!(ctx.first_language, "Indonesian");
        assert!(ctx.tutor_role.starts_with("Sam"));
        assert_eq!(ctx.goals.len(), 5);
        assert!(matches!(&ctx.focus, Focus::Unit(title) if title == "Hello! Nice to meet you"));
        assert_eq!(scenario.max_turns, Some(8));
        // Vocabulary lemmas first, then the grammar patterns of the roleplay.
        assert!(scenario.target_language.contains(&"hello".to_owned()));
        assert!(
            scenario
                .target_language
                .iter()
                .any(|t| t.contains("I'm from"))
        );
        let ids: Vec<&str> = scenario.objectives.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["o1-greet", "o2-introduce", "o3-understand"]);
    }

    #[test]
    fn the_mode_can_be_overridden_and_an_unknown_activity_is_refused() {
        let unit = example_unit();
        let accuracy = Scenario::from_unit(&unit, None, Some(FeedbackMode::Accuracy), "Indonesian")
            .expect("scenario");
        assert_eq!(accuracy.context.mode, FeedbackMode::Accuracy);
        let named = Scenario::from_unit(&unit, Some("a11-roleplay-classmate"), None, "Indonesian");
        assert!(named.is_ok());
        // A matching id that is not a roleplay is no scenario either.
        for id in ["no-such-activity", "a10-speak-introduce"] {
            let refused = Scenario::from_unit(&unit, Some(id), None, "Indonesian");
            assert!(
                matches!(refused, Err(VoiceError::NoScenario(Some(_)))),
                "{id}"
            );
        }
    }

    #[test]
    fn the_default_settings_are_valid_and_a_zero_capacity_is_refused() {
        assert!(VoiceConfig::default().validate().is_ok());
        let cases: [fn(&mut VoiceConfig); 5] = [
            |c| c.frame_queue = 0,
            |c| c.inbox = 0,
            |c| c.event_capacity = 0,
            |c| c.frame_len = 256,
            |c| c.output_poll = Duration::ZERO,
        ];
        for break_it in cases {
            let mut config = VoiceConfig::default();
            break_it(&mut config);
            assert!(config.validate().is_err());
        }
    }
}
