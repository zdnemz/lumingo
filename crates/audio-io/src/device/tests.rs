use super::*;
use crate::fake::FakeBackend;

const FORMAT: StreamFormat = StreamFormat {
    sample_rate: 48_000,
    channels: 2,
};

struct Rig {
    backend: FakeBackend,
    speakers: DeviceId,
    headset: DeviceId,
    mic: DeviceId,
}

fn rig() -> Rig {
    let backend = FakeBackend::new();
    let speakers = backend.add_device("Speakers", Direction::Output, FORMAT, true);
    let headset = backend.add_device("USB Headset", Direction::Output, FORMAT, false);
    let mic = backend.add_device("Built-in Mic", Direction::Input, FORMAT, true);
    Rig {
        backend,
        speakers,
        headset,
        mic,
    }
}

fn registry(rig: &Rig, store: impl PrefsStore + 'static) -> DeviceRegistry {
    DeviceRegistry::new(Arc::new(rig.backend.clone()), Box::new(store))
}

fn temp_path(tag: &str) -> PathBuf {
    let unique = format!(
        "lumingo-audio-io-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    );
    std::env::temp_dir().join(unique).join("devices.json")
}

fn cleanup(path: &std::path::Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn remembered_input(id: &str, name: &str) -> MemoryPrefs {
    let prefs = MemoryPrefs::default();
    prefs
        .save(&DevicePrefs {
            input: Some(DeviceRef {
                id: DeviceId(id.to_owned()),
                name: name.to_owned(),
            }),
            output: None,
        })
        .expect("memory store saves");
    prefs
}

#[test]
fn lists_only_devices_of_the_asked_direction() {
    let rig = rig();
    let registry = registry(&rig, MemoryPrefs::default());
    let outputs = registry.list(Direction::Output).expect("lists");
    assert_eq!(outputs.len(), 2);
    assert!(outputs.iter().all(|d| d.direction == Direction::Output));
    assert_eq!(registry.list(Direction::Input).expect("lists").len(), 1);
}

#[test]
fn with_nothing_chosen_the_system_default_is_used() {
    let rig = rig();
    let registry = registry(&rig, MemoryPrefs::default());
    let resolved = registry.resolve(Direction::Output).expect("resolves");
    assert_eq!(resolved.info.id, rig.speakers);
    assert_eq!(resolved.choice, Choice::Default);
}

#[test]
fn a_selection_is_used_and_survives_a_restart() {
    let rig = rig();
    let path = temp_path("restart");
    let first = registry(&rig, FilePrefs::new(&path));
    first
        .select(Direction::Output, Some(&rig.headset))
        .expect("selects");
    drop(first);

    // A new registry on the same file is a new run of the app.
    let again = registry(&rig, FilePrefs::new(&path));
    let resolved = again.resolve(Direction::Output).expect("resolves");
    assert_eq!(resolved.info.id, rig.headset);
    assert_eq!(resolved.choice, Choice::Selected);
    assert!(!path.with_extension("json.tmp").exists());
    cleanup(&path);
}

#[test]
fn choosing_a_device_that_is_not_there_is_refused_and_changes_nothing() {
    let rig = rig();
    let registry = registry(&rig, MemoryPrefs::default());
    let ghost = DeviceId("fake:Output:Nothing".to_owned());
    let err = registry
        .select(Direction::Output, Some(&ghost))
        .expect_err("refused");
    assert!(matches!(err, DeviceError::NotFound { .. }));
    assert_eq!(registry.remembered(Direction::Output), None);
}

#[test]
fn a_missing_remembered_device_falls_back_to_the_default_but_stays_remembered() {
    let rig = rig();
    let registry = registry(&rig, MemoryPrefs::default());
    registry
        .select(Direction::Output, Some(&rig.headset))
        .expect("selects");
    rig.backend.unplug(&rig.headset);

    let resolved = registry.resolve(Direction::Output).expect("resolves");
    assert_eq!(resolved.info.id, rig.speakers);
    assert_eq!(
        resolved.choice,
        Choice::DefaultInstead {
            missing: "USB Headset".to_owned()
        }
    );
    assert!(registry.remembered(Direction::Output).is_some());

    // Plugged back in, the remembered pick is used again without a new choice.
    rig.backend
        .add_device("USB Headset", Direction::Output, FORMAT, false);
    let resolved = registry.resolve(Direction::Output).expect("resolves");
    assert_eq!(resolved.info.id, rig.headset);
    assert_eq!(resolved.choice, Choice::Selected);
}

#[test]
fn a_device_whose_id_changed_is_found_again_by_its_unique_name() {
    let rig = rig();
    let registry = DeviceRegistry::new(
        Arc::new(rig.backend.clone()),
        Box::new(remembered_input("old-port-id", "Studio Mic")),
    );
    rig.backend.add_device_with_id(
        DeviceId("new-port-id".to_owned()),
        "Studio Mic",
        Direction::Input,
        FORMAT,
        false,
    );
    let resolved = registry.resolve(Direction::Input).expect("resolves");
    assert_eq!(resolved.info.id, DeviceId("new-port-id".to_owned()));
    assert_eq!(resolved.choice, Choice::Selected);
}

#[test]
fn two_devices_with_the_remembered_name_are_not_guessed_between() {
    let rig = rig();
    let registry = DeviceRegistry::new(
        Arc::new(rig.backend.clone()),
        Box::new(remembered_input("gone", "Twin Mic")),
    );
    for id in ["twin-a", "twin-b"] {
        rig.backend.add_device_with_id(
            DeviceId(id.to_owned()),
            "Twin Mic",
            Direction::Input,
            FORMAT,
            false,
        );
    }
    let resolved = registry.resolve(Direction::Input).expect("resolves");
    assert_eq!(resolved.info.id, rig.mic, "falls back to the default");
    assert_eq!(
        resolved.choice,
        Choice::DefaultInstead {
            missing: "Twin Mic".to_owned()
        }
    );
}

#[test]
fn no_devices_at_all_is_a_typed_error() {
    let backend = FakeBackend::new();
    let registry = DeviceRegistry::new(Arc::new(backend), Box::new(MemoryPrefs::default()));
    assert_eq!(
        registry.resolve(Direction::Input),
        Err(DeviceError::NoDevice(Direction::Input))
    );
}

#[test]
fn going_back_to_the_default_clears_the_choice() {
    let rig = rig();
    let registry = registry(&rig, MemoryPrefs::default());
    registry
        .select(Direction::Output, Some(&rig.headset))
        .expect("selects");
    registry.select(Direction::Output, None).expect("clears");
    assert_eq!(registry.remembered(Direction::Output), None);
}

#[test]
fn a_damaged_preferences_file_means_defaults_not_a_crash() {
    let rig = rig();
    let path = temp_path("damaged");
    std::fs::create_dir_all(path.parent().expect("has a parent")).expect("dir");
    std::fs::write(&path, b"{ not json").expect("writes");
    let registry = registry(&rig, FilePrefs::new(&path));
    assert_eq!(registry.remembered(Direction::Output), None);
    assert!(registry.resolve(Direction::Output).is_ok());
    cleanup(&path);
}

#[test]
fn errors_read_as_plain_sentences() {
    let lost = DeviceError::Lost {
        direction: Direction::Input,
        name: "USB Headset".to_owned(),
    };
    assert_eq!(lost.to_string(), "the microphone \"USB Headset\" was lost");
    assert_eq!(
        DeviceError::NoDevice(Direction::Output).to_string(),
        "no speaker is available"
    );
}
