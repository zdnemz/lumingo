//! Peak resident memory of this process, sampled from a background thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use sysinfo::{ProcessesToUpdate, System, get_current_pid};

const POLL: Duration = Duration::from_millis(10);

/// Starts polling when created and reports the highest value seen when stopped.
pub struct PeakMemorySampler {
    stop: Arc<AtomicBool>,
    peak_bytes: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

fn current_rss_bytes(system: &mut System) -> Option<u64> {
    let pid = get_current_pid().ok()?;
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(sysinfo::Process::memory)
}

impl PeakMemorySampler {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak_bytes = Arc::new(AtomicU64::new(0));
        let thread = {
            let stop = Arc::clone(&stop);
            let peak = Arc::clone(&peak_bytes);
            std::thread::spawn(move || {
                let mut system = System::new();
                while !stop.load(Ordering::Acquire) {
                    if let Some(bytes) = current_rss_bytes(&mut system) {
                        peak.fetch_max(bytes, Ordering::AcqRel);
                    }
                    std::thread::sleep(POLL);
                }
                // One last reading so a short run still reports something.
                if let Some(bytes) = current_rss_bytes(&mut system) {
                    peak.fetch_max(bytes, Ordering::AcqRel);
                }
            })
        };
        Self {
            stop,
            peak_bytes,
            thread: Some(thread),
        }
    }

    /// Stops the sampler and returns the peak in MB, or `None` if the platform
    /// gave no reading at all.
    pub fn stop(mut self) -> Option<f64> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let bytes = self.peak_bytes.load(Ordering::Acquire);
        (bytes > 0).then(|| bytes as f64 / (1024.0 * 1024.0))
    }
}

impl Drop for PeakMemorySampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_peak_rises_when_memory_is_used() {
        let sampler = PeakMemorySampler::start();
        std::thread::sleep(Duration::from_millis(60));
        // 96 MB, touched page by page so the operating system really commits it.
        let mut block = vec![0_u8; 96 * 1024 * 1024];
        for index in (0..block.len()).step_by(4096) {
            block[index] = 1;
        }
        std::thread::sleep(Duration::from_millis(120));
        let held = block
            .iter()
            .step_by(4096)
            .map(|b| u64::from(*b))
            .sum::<u64>();
        let peak = sampler.stop().expect("a reading");
        drop(block);
        assert!(held > 0);
        assert!(
            peak >= 90.0,
            "peak {peak} MB should include the 96 MB block"
        );
    }

    #[test]
    fn stopping_twice_in_a_row_is_not_possible_and_drop_is_safe() {
        let sampler = PeakMemorySampler::start();
        drop(sampler);
    }
}
