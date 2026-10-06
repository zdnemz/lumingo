//! The session phase, kept by the tutor engine's own state machine.
//!
//! The voice loop does not have a second state machine. [`PhaseCell`] wraps
//! `tutor_engine::Session` (a voice conversation) and adds two things the loop
//! needs: an epoch, so a turn task that was cancelled cannot move the session
//! after the learner has already moved on, and an event for every change.

use std::sync::{Mutex, MutexGuard, PoisonError};

use tutor_engine::{Event, Phase, Session, SessionKind, TransitionError};

use super::event::{EventSink, VoiceEvent};

struct Inner {
    session: Session,
    /// Changes whenever a turn begins or is cancelled. Work that belongs to an
    /// older epoch is stale.
    epoch: u64,
    /// Turns started, for display. Cancelled turns count.
    turns_started: u64,
}

pub(crate) struct PhaseCell {
    inner: Mutex<Inner>,
    events: EventSink,
}

impl PhaseCell {
    pub(crate) fn new(events: EventSink) -> Self {
        Self {
            inner: Mutex::new(Inner {
                session: Session::new(SessionKind::Conversation, tutor_engine::Channel::Voice),
                epoch: 0,
                turns_started: 0,
            }),
            events,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // The state is a small value; a panic elsewhere cannot leave it half written.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn phase(&self) -> Phase {
        self.lock().session.phase()
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.lock().epoch
    }

    pub(crate) fn turns_completed(&self) -> u32 {
        self.lock().session.turns_completed()
    }

    /// The number of the turn that began last, 0 before the first.
    pub(crate) fn turns_started(&self) -> u64 {
        self.lock().turns_started
    }

    /// Starts a turn: a new epoch and the next display number.
    pub(crate) fn begin_turn(&self) -> (u64, u64) {
        let mut inner = self.lock();
        inner.epoch += 1;
        inner.turns_started += 1;
        (inner.epoch, inner.turns_started)
    }

    /// Makes everything that belongs to the current epoch stale.
    pub(crate) fn invalidate(&self) -> u64 {
        let mut inner = self.lock();
        inner.epoch += 1;
        inner.epoch
    }

    /// Applies an event whatever the epoch. Used by the orchestrator, which
    /// owns the session.
    pub(crate) fn apply(&self, event: Event) -> Result<Phase, TransitionError> {
        let mut inner = self.lock();
        let result = inner.session.apply(event);
        match &result {
            Ok(phase) => self.events.publish(VoiceEvent::State(*phase)),
            Err(error) => tracing::warn!(%error, "the voice session refused an event"),
        }
        result
    }

    /// Applies an event for a turn task, only while its epoch is current.
    pub(crate) fn apply_for(&self, epoch: u64, event: Event) -> Option<Phase> {
        let mut inner = self.lock();
        if inner.epoch != epoch {
            return None;
        }
        match inner.session.apply(event) {
            Ok(phase) => {
                self.events.publish(VoiceEvent::State(phase));
                Some(phase)
            }
            Err(error) => {
                tracing::warn!(%error, "the voice session refused an event");
                None
            }
        }
    }
}
