#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
// Each test binary uses a different subset of these helpers.
#![allow(dead_code)]

pub mod capture;
pub mod fake_provider;
pub mod seed;
pub mod voice;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use app_core::api::HardwareProfile;
use app_core::config::CoreConfig;
use app_core::error::CoreResult;
use app_core::{AppCore, Clock};
use storage::{LocalDate, Timestamp};
use tempfile::TempDir;

/// A clock the test moves by hand.
#[derive(Debug)]
pub struct ManualClock {
    state: Mutex<(Timestamp, LocalDate)>,
}

impl ManualClock {
    pub fn new(now: &str, today: &str) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new((
                Timestamp::parse(now).expect("timestamp"),
                LocalDate::parse(today).expect("date"),
            )),
        })
    }

    pub fn set(&self, now: &str, today: &str) {
        *self.state.lock().unwrap() = (
            Timestamp::parse(now).expect("timestamp"),
            LocalDate::parse(today).expect("date"),
        );
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        self.state.lock().unwrap().0
    }

    fn today(&self) -> CoreResult<LocalDate> {
        Ok(self.state.lock().unwrap().1)
    }
}

pub struct TestCore {
    pub core: Arc<AppCore>,
    pub dir: TempDir,
    pub clock: Arc<ManualClock>,
}

impl TestCore {
    pub fn clock_now(&self) -> Timestamp {
        self.clock.now()
    }
}

/// The hardware every test core reports, so no test depends on the machine.
pub fn fixed_hardware() -> HardwareProfile {
    app_core::hardware::assess(Some(16_000_000_000), Some(8))
}

/// A config over a temporary directory with no `.env` files, an empty process
/// environment and a fixed clock.
pub fn config(dir: &TempDir, clock: &Arc<ManualClock>, env: &[(&str, &str)]) -> CoreConfig {
    let mut config = CoreConfig::new(dir.path().join("data"), dir.path().join("units"));
    config.env_profiles = llm_client::EnvProfileLoader::new(Vec::new());
    let vars: HashMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    config.env_lookup = Arc::new(move |name| vars.get(name).cloned());
    config.clock = clock.clone();
    config.hardware = Some(fixed_hardware());
    config
}

pub async fn test_core() -> TestCore {
    test_core_with_env(&[]).await
}

pub async fn test_core_with_env(env: &[(&str, &str)]) -> TestCore {
    let dir = tempfile::tempdir().expect("temp dir");
    let clock = ManualClock::new("2026-10-05T08:00:00.000Z", "2026-10-05");
    let core = AppCore::open(config(&dir, &clock, env))
        .await
        .expect("open the core");
    TestCore { core, dir, clock }
}
