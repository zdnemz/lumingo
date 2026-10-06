/// Half-duplex microphone gate (context_pack.md section 4, step 3). Frames are
/// dropped before the VAD while the tutor is audible and for a hold-off after,
/// so the tutor never hears itself. It counts frames, not time, so it is
/// deterministic in tests.
#[derive(Debug)]
pub struct MicGate {
    holdoff_frames: u32,
    remaining: u32,
}

impl MicGate {
    /// `holdoff_ms` is 150 in the spec. It is rounded up to whole frames.
    pub fn new(holdoff_ms: u32, frame_ms: u32) -> Self {
        Self {
            holdoff_frames: holdoff_ms.div_ceil(frame_ms.max(1)),
            remaining: 0,
        }
    }

    /// Call once per captured frame. Returns true when the frame may go on to the VAD.
    pub fn pass(&mut self, playback_active: bool) -> bool {
        if playback_active {
            self.remaining = self.holdoff_frames;
            return false;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
            return false;
        }
        true
    }

    /// Push-to-talk bypasses the gate: the learner decided to speak, so the next
    /// frame passes even inside the hold-off. Playback still blocks it.
    pub fn open(&mut self) {
        self.remaining = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_pass_when_nothing_plays() {
        let mut g = MicGate::new(150, 32);
        assert!((0..10).all(|_| g.pass(false)));
    }

    #[test]
    fn frames_are_dropped_during_playback_and_for_the_holdoff_after() {
        let mut g = MicGate::new(150, 32); // 150 ms is 5 frames of 32 ms
        assert!(!g.pass(true) && !g.pass(true));
        let after: Vec<bool> = (0..7).map(|_| g.pass(false)).collect();
        assert_eq!(after, [false, false, false, false, false, true, true]);
    }

    #[test]
    fn open_ends_the_holdoff_early_but_not_playback() {
        let mut g = MicGate::new(150, 32);
        g.pass(true);
        g.open();
        assert!(g.pass(false));
        g.pass(true);
        g.open();
        assert!(!g.pass(true));
    }
}
