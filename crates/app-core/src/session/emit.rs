//! Publishing the events of a session and keeping the snapshot's picture of it.
//!
//! A session keeps one [`ActiveSessionView`] in memory, which the snapshot reads.
//! Every change goes through [`Emitter`]: it updates the view first and then
//! publishes the event, so a snapshot taken between the two is at worst a little
//! ahead of the stream, and the client applies the event to it harmlessly: every
//! event is keyed (by turn number or stored position) and applying one twice
//! changes nothing.
//!
//! The view holds at most [`RECENT_LINES`] lines of at most [`RECENT_LINE_CHARS`]
//! characters, and the partial reply at most [`PENDING_REPLY_CHARS`] characters:
//! the snapshot must stay small, because a lagging client is sent one every time.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::api::{
    ActiveSessionView, ErrorCode, FeedbackView, LineRole, PronFindingsView, RECENT_LINE_CHARS,
    RECENT_LINES, ServerEvent, SessionChannel, SessionKind, SessionLife, TurnAnalysisView,
    TurnLine, TurnPhase,
};
use crate::events::EventBus;

/// The longest partial reply the snapshot holds, in characters.
pub const PENDING_REPLY_CHARS: usize = 4_000;

fn cut(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    text.chars().take(limit).collect()
}

#[derive(Clone)]
pub(crate) struct Emitter {
    bus: Arc<EventBus>,
    id: i64,
    kind: SessionKind,
    view: Arc<Mutex<ActiveSessionView>>,
}

impl Emitter {
    pub(crate) fn new(bus: Arc<EventBus>, view: ActiveSessionView) -> Self {
        Self {
            bus,
            id: view.id,
            kind: view.kind,
            view: Arc::new(Mutex::new(view)),
        }
    }

    pub(crate) fn id(&self) -> i64 {
        self.id
    }

    fn lock(&self) -> MutexGuard<'_, ActiveSessionView> {
        // The view is replaced field by field and is valid after any of them.
        self.view.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A copy of the view, for the snapshot and for answers.
    pub(crate) fn view(&self) -> ActiveSessionView {
        self.lock().clone()
    }

    /// Like [`Emitter::session_state`], and nothing when the session is in that
    /// state already: a state that two places announce is sent once.
    pub(crate) fn session_state_if_changed(
        &self,
        life: SessionLife,
        fault: Option<crate::api::EngineFaultView>,
        message: Option<String>,
    ) {
        {
            let view = self.lock();
            if view.life == life && view.fault == fault {
                return;
            }
        }
        self.session_state(life, fault, message);
    }

    pub(crate) fn session_state(
        &self,
        life: SessionLife,
        fault: Option<crate::api::EngineFaultView>,
        message: Option<String>,
    ) {
        {
            let mut view = self.lock();
            view.life = life;
            view.fault = fault;
            if life != SessionLife::Active {
                view.turn_state = None;
            }
        }
        self.bus.publish(|seq| ServerEvent::SessionState {
            seq,
            session_id: self.id,
            kind: self.kind,
            life,
            fault,
            message,
        });
    }

    /// The turn moved to `state`. `tutor_turn_seq` is set on the move that ends a
    /// reply.
    pub(crate) fn turn_state(&self, turn: u64, state: TurnPhase, tutor_turn_seq: Option<i64>) {
        {
            let mut view = self.lock();
            view.life = SessionLife::Active;
            view.fault = None;
            view.turn_state = Some(state);
            view.turn = turn;
            if tutor_turn_seq.is_some() {
                view.turns_completed = view.turns_completed.saturating_add(1);
            }
        }
        self.bus.publish(|seq| ServerEvent::TurnState {
            seq,
            session_id: self.id,
            turn,
            state,
            tutor_turn_seq,
        });
    }

    /// The learner's line of a turn. A line with the same turn number replaces the
    /// earlier one: a spoken turn is announced when heard and again when stored.
    pub(crate) fn learner_line(
        &self,
        turn: u64,
        text: &str,
        source: SessionChannel,
        turn_seq: Option<i64>,
        edited: bool,
    ) {
        {
            let mut view = self.lock();
            // A correction names the turn by its number and has no stored position of
            // its own: the line keeps the one it had.
            let known = view
                .recent
                .iter()
                .find(|l| l.turn == turn && l.role == LineRole::Learner)
                .and_then(|l| l.seq);
            upsert(
                &mut view.recent,
                TurnLine {
                    turn,
                    role: LineRole::Learner,
                    text: cut(text, RECENT_LINE_CHARS),
                    seq: turn_seq.or(known),
                    edited,
                },
            );
        }
        self.bus.publish(|seq| ServerEvent::TranscriptFinal {
            seq,
            session_id: self.id,
            turn,
            text: text.to_owned(),
            source,
            turn_seq,
            edited,
        });
    }

    pub(crate) fn text_delta(&self, turn: u64, delta: &str) {
        {
            let mut view = self.lock();
            let pending = view.pending_reply.get_or_insert_with(String::new);
            if pending.chars().count() < PENDING_REPLY_CHARS {
                pending.push_str(delta);
            }
        }
        self.bus.publish(|seq| ServerEvent::TutorTextDelta {
            seq,
            session_id: self.id,
            turn,
            delta: delta.to_owned(),
        });
    }

    pub(crate) fn sentence_spoken(&self, turn: u64, index: u32, text: &str) {
        {
            let mut view = self.lock();
            let pending = view.pending_reply.get_or_insert_with(String::new);
            if pending.chars().count() < PENDING_REPLY_CHARS {
                if !pending.is_empty() {
                    pending.push(' ');
                }
                pending.push_str(text);
            }
        }
        self.bus.publish(|seq| ServerEvent::TutorSentenceSpoken {
            seq,
            session_id: self.id,
            turn,
            index,
            text: text.to_owned(),
        });
    }

    /// The tutor's reply is complete and stored (or kept as far as it went): it
    /// becomes a line, and the partial reply is cleared.
    pub(crate) fn tutor_line(&self, turn: u64, text: &str, turn_seq: Option<i64>) {
        let mut view = self.lock();
        view.pending_reply = None;
        if !text.is_empty() {
            upsert(
                &mut view.recent,
                TurnLine {
                    turn,
                    role: LineRole::Tutor,
                    text: cut(text, RECENT_LINE_CHARS),
                    seq: turn_seq,
                    edited: false,
                },
            );
        }
    }

    pub(crate) fn clear_pending_reply(&self) {
        self.lock().pending_reply = None;
    }

    /// Takes the partial reply, leaving none.
    pub(crate) fn take_pending_reply(&self) -> Option<String> {
        self.lock().pending_reply.take()
    }

    /// For a session whose engine counts the turns itself.
    pub(crate) fn set_turns_completed(&self, turns: u32) {
        self.lock().turns_completed = turns;
    }

    pub(crate) fn set_activity(&self, activity_id: Option<String>) {
        self.lock().activity_id = activity_id;
    }

    pub(crate) fn analysis(&self, analysis: TurnAnalysisView) {
        self.bus.publish(|seq| ServerEvent::AnalysisReady {
            seq,
            session_id: self.id,
            analysis,
        });
    }

    pub(crate) fn feedback(&self, feedback: FeedbackView) {
        self.bus.publish(|seq| ServerEvent::FeedbackReady {
            seq,
            session_id: self.id,
            feedback,
        });
    }

    pub(crate) fn pron(&self, activity_id: &str, findings: PronFindingsView) {
        self.bus.publish(|seq| ServerEvent::PronFindings {
            seq,
            session_id: self.id,
            activity_id: activity_id.to_owned(),
            findings,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn latency(
        &self,
        turn: u64,
        parts_ms: [Option<f64>; 5],
        sum_ms: f64,
        complete: bool,
    ) {
        let [
            endpointing_wait_ms,
            stt_finalise_ms,
            llm_first_sentence_ms,
            tts_first_sentence_ms,
            output_start_ms,
        ] = parts_ms;
        self.bus.publish(|seq| ServerEvent::LatencyReport {
            seq,
            session_id: self.id,
            turn,
            endpointing_wait_ms,
            stt_finalise_ms,
            llm_first_sentence_ms,
            tts_first_sentence_ms,
            output_start_ms,
            sum_ms,
            complete,
        });
    }

    pub(crate) fn error(&self, code: ErrorCode, message: &str) {
        self.bus.publish(|seq| ServerEvent::Error {
            seq,
            session_id: Some(self.id),
            code,
            message: message.to_owned(),
        });
    }
}

/// Puts `line` in the list, replacing the line of the same turn and role, and
/// keeps the newest [`RECENT_LINES`].
fn upsert(lines: &mut Vec<TurnLine>, line: TurnLine) {
    match lines
        .iter_mut()
        .find(|l| l.turn == line.turn && l.role == line.role)
    {
        Some(existing) => *existing = line,
        None => lines.push(line),
    }
    if lines.len() > RECENT_LINES {
        let excess = lines.len() - RECENT_LINES;
        lines.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::SessionChannel;

    fn emitter() -> (Emitter, tokio::sync::broadcast::Receiver<ServerEvent>) {
        let bus = Arc::new(EventBus::new());
        let rx = bus.subscribe();
        let view = ActiveSessionView {
            id: 7,
            kind: SessionKind::TextChat,
            unit_id: None,
            channel: SessionChannel::Text,
            life: SessionLife::Active,
            turn_state: Some(TurnPhase::Waiting),
            fault: None,
            turns_completed: 0,
            turn: 0,
            recent: Vec::new(),
            pending_reply: None,
            speaking: false,
            activity_id: None,
        };
        (Emitter::new(bus, view), rx)
    }

    #[test]
    fn lines_are_keyed_by_turn_and_role_bounded_and_cut() {
        let (emit, _rx) = emitter();
        emit.learner_line(1, "hello", SessionChannel::Voice, None, false);
        emit.learner_line(1, "hello there", SessionChannel::Voice, Some(2), false);
        let view = emit.view();
        assert_eq!(view.recent.len(), 1, "the second line replaces the first");
        assert_eq!(view.recent[0].text, "hello there");
        assert_eq!(view.recent[0].seq, Some(2));

        emit.learner_line(1, "hello there, friend", SessionChannel::Voice, None, true);
        let view = emit.view();
        assert_eq!(view.recent[0].text, "hello there, friend");
        assert_eq!(
            view.recent[0].seq,
            Some(2),
            "a correction keeps the stored position"
        );
        assert!(view.recent[0].edited);

        for turn in 2..=30 {
            emit.learner_line(turn, "x", SessionChannel::Text, None, false);
        }
        let view = emit.view();
        assert_eq!(view.recent.len(), RECENT_LINES);
        assert_eq!(view.recent.last().map(|l| l.turn), Some(30));

        emit.learner_line(
            99,
            &"é".repeat(RECENT_LINE_CHARS + 50),
            SessionChannel::Text,
            None,
            false,
        );
        let longest = emit.view().recent.last().map(|l| l.text.chars().count());
        assert_eq!(longest, Some(RECENT_LINE_CHARS));
    }

    #[test]
    fn the_partial_reply_is_bounded_and_cleared_by_the_stored_line() {
        let (emit, _rx) = emitter();
        for _ in 0..(PENDING_REPLY_CHARS / 100 + 20) {
            emit.text_delta(1, &"a".repeat(100));
        }
        let pending = emit.view().pending_reply.expect("partial reply");
        assert!(pending.chars().count() <= PENDING_REPLY_CHARS + 100);
        emit.tutor_line(1, "done", Some(3));
        let view = emit.view();
        assert_eq!(view.pending_reply, None);
        assert_eq!(view.recent.last().map(|l| l.role), Some(LineRole::Tutor));
    }

    #[test]
    fn a_turn_that_ends_a_reply_counts_and_a_pause_clears_the_turn_state() {
        let (emit, mut rx) = emitter();
        emit.turn_state(1, TurnPhase::Thinking, None);
        emit.turn_state(1, TurnPhase::Waiting, Some(2));
        assert_eq!(emit.view().turns_completed, 1);
        emit.session_state(SessionLife::Paused, None, None);
        let view = emit.view();
        assert_eq!(view.life, SessionLife::Paused);
        assert_eq!(view.turn_state, None);
        let names: Vec<&str> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|e| e.name())
            .collect();
        assert_eq!(names, ["TurnState", "TurnState", "SessionState"]);
    }
}
