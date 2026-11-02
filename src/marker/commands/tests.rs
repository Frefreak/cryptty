use super::*;

#[test]
fn fish_history_keeps_names_without_arguments_or_metadata() {
    let history = br#"- cmd: very-long-custom-command secret-value
  when: 123
  paths:
    - /private/data
- cmd: '/opt/tools/another-long-command' 'argument with spaces'
- cmd: echo first\nvery-secret-second-word
- cmd: printf 'escaped\\value'
- cmd: VAR=/secret/value tool
- cmd: https://host/secret
- cmd: bad|syntax
- cmd: unfinished 'quote
"#;
    assert_eq!(
        fish_names(history),
        HashSet::from([
            "very-long-custom-command".into(),
            "another-long-command".into(),
            "echo".into(),
            "printf".into(),
        ])
    );
}

#[test]
fn refresh_skips_unchanged_history_and_replaces_cached_names() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fish_history");
    let cache = directory.path().join("cache/names.txt");
    std::fs::write(&path, "- cmd: first-tool secret\n").unwrap();
    let mut source = Source {
        path: path.clone(),
        cache: Some(cache.clone()),
        stamp: None,
    };
    let names = source.refresh().unwrap().unwrap();
    assert_eq!(names, HashSet::from(["first-tool".into()]));
    assert_eq!(std::fs::read_to_string(&cache).unwrap(), "first-tool\n");
    assert_eq!(source.load_cache(), Some(names));
    assert!(source.refresh().unwrap().is_none());
    std::fs::write(&path, "- cmd: replacement-tool other-secret\n").unwrap();
    assert_eq!(
        source.refresh().unwrap().unwrap(),
        HashSet::from(["replacement-tool".into()])
    );
    std::fs::remove_file(&path).unwrap();
    assert!(source.refresh().is_err());
    assert_eq!(
        source.load_cache().unwrap(),
        HashSet::from(["replacement-tool".into()])
    );
    std::fs::write(&path, "").unwrap();
    assert!(source.refresh().unwrap().unwrap().is_empty());
}

#[test]
fn bounded_reads_drop_partial_records_and_reject_directories() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history");
    std::fs::write(&path, "- cmd: old-tool\n- cmd: new-tool\n").unwrap();
    let bytes = read_tail(&path, 20).unwrap();
    assert!(bytes.len() <= 20);
    assert_eq!(fish_names(&bytes), HashSet::from(["new-tool".into()]));
    assert!(read_tail(directory.path(), 100).is_err());
}

#[test]
fn background_refresh_waits_and_stops_without_waiting_for_interval() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history");
    std::fs::write(&path, "- cmd: worker-learned-command arg\n").unwrap();
    let (stop, receiver) = mpsc::channel();
    let output = Arc::new(RwLock::new(Arc::new(HashSet::new())));
    let observed = output.clone();
    let source = Source {
        path: path.clone(),
        cache: None,
        stamp: None,
    };
    let handle = std::thread::spawn(move || {
        refresh_loop(source, Duration::from_secs(600), receiver, output);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !observed.read().unwrap().contains("worker-learned-command") {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let frozen = observed.read().unwrap().clone();
    std::fs::write(path, "- cmd: another-command arg\n").unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(Arc::ptr_eq(&frozen, &observed.read().unwrap()));
    drop(stop);
    while !handle.is_finished() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    handle.join().unwrap();
}

#[test]
fn periodic_refresh_publishes_new_names_without_changing_old_snapshots() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history");
    std::fs::write(&path, "- cmd: initial-tool arg\n").unwrap();
    let (stop, receiver) = mpsc::channel();
    let output = Arc::new(RwLock::new(Arc::new(HashSet::new())));
    let observed = output.clone();
    let source = Source {
        path: path.clone(),
        cache: None,
        stamp: None,
    };
    let handle = std::thread::spawn(move || {
        refresh_loop(source, Duration::from_millis(20), receiver, output);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let wait_for = |name| {
        while !observed.read().unwrap().contains(name) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    wait_for("initial-tool");
    let frozen = observed.read().unwrap().clone();
    std::fs::write(&path, "- cmd: refreshed-long-command other\n").unwrap();
    wait_for("refreshed-long-command");
    assert!(frozen.contains("initial-tool"));
    assert!(!frozen.contains("refreshed-long-command"));
    assert!(!observed.read().unwrap().contains("initial-tool"));
    std::fs::remove_file(&path).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert!(observed.read().unwrap().contains("refreshed-long-command"));
    drop(stop);
    handle.join().unwrap();
}

#[test]
fn history_settings_default_to_ten_minutes_and_reject_invalid_values() {
    let config = crate::config::Config::default();
    assert_eq!(
        config
            .markers
            .command_history
            .refresh_interval_seconds
            .get(),
        600
    );
    assert!(config.markers.command_history.enabled);
    for yaml in [
        "markers: {command_history: {refresh_interval_seconds: 0}}",
        "markers: {command_history: {shell: unknown}}",
        "markers: {command_name_max_length: 0}",
    ] {
        assert!(crate::config::Config::parse(yaml).is_err());
    }
    let config = crate::config::Config::parse(
        "markers: {command_history: {enabled: false, refresh_interval_seconds: 1200}}"
    ).unwrap();
    assert_eq!(
        config
            .markers
            .command_history
            .refresh_interval_seconds
            .get(),
        1200
    );
    assert!(Source::new(&config.markers.command_history).is_none());
}
