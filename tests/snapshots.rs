use leadline::snapshots::{SNAPSHOT_SCHEMA_VERSION, SnapshotOutcome, TrendPoint, TrendStore};

fn point(commit: &str, fingerprint: &str) -> TrendPoint {
    TrendPoint {
        commit: commit.to_owned(),
        commit_timestamp: 100,
        scope: ".".to_owned(),
        analyzer_version: "test".to_owned(),
        config_hash: "config".to_owned(),
        mailmap_hash: None,
        metric_profile: "default-v1".to_owned(),
        risk_model: "change-risk-v1".to_owned(),
        duplication_profile: "tokens-v1".to_owned(),
        mutation_model: "none".to_owned(),
        source_hash: "source".to_owned(),
        input_fingerprint: fingerprint.to_owned(),
        files: 1,
        functions: 2,
        parse_errors: 0,
        mean_cognitive: Some(5.0),
        max_cognitive: Some(10),
        coverage_percent: None,
        hotspot_files: None,
        max_hotspot_score: None,
        risk_files_ge_70: 0,
        max_risk_score: None,
        dependency_edges: 0,
        cycles: 0,
        coupling_edges: None,
        max_ownership_concentration_percent: None,
        mutation_score: None,
        scored_mutants: None,
        duplication_complete: true,
        duplicate_groups: Some(0),
        duplicated_lines: Some(0),
        policy_info: 0,
        policy_warning: 0,
        policy_error: 0,
    }
}

#[test]
fn append_is_idempotent_and_replaces_only_with_consent() {
    let mut store = TrendStore::new();
    assert_eq!(
        store.append(point("a", "one"), false).unwrap(),
        SnapshotOutcome::Added
    );
    assert_eq!(
        store.append(point("a", "one"), false).unwrap(),
        SnapshotOutcome::Unchanged
    );
    assert!(store.append(point("a", "two"), false).is_err());
    assert_eq!(
        store.append(point("a", "two"), true).unwrap(),
        SnapshotOutcome::Replaced
    );
    assert_eq!(store.points.len(), 1);
    assert_eq!(store.points[0].input_fingerprint, "two");
    assert_eq!(store.schema_version, SNAPSHOT_SCHEMA_VERSION);
}

#[test]
fn different_models_are_separate_series() {
    let mut store = TrendStore::new();
    store.append(point("a", "one"), false).unwrap();
    let mut other = point("a", "one");
    other.risk_model = "change-risk-v2".to_owned();
    assert_eq!(store.append(other, false).unwrap(), SnapshotOutcome::Added);
    assert_eq!(store.points.len(), 2);
}

#[test]
fn store_round_trips_with_lock_cleanup() {
    let root = std::env::temp_dir().join(format!(
        "leadline-snapshot-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("trends.json");
    let mut store = TrendStore::new();
    store.append(point("a", "one"), false).unwrap();
    store.append(point("b", "two"), false).unwrap();
    store.write(&path).unwrap();

    let read = TrendStore::read(&path).unwrap();
    assert_eq!(read, store);
    assert!(
        !root.join(".trends.json.lock").exists(),
        "lock is released after write"
    );
    assert!(
        std::fs::read_dir(&root).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")),
        "no temp file is left behind"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn read_rejects_unknown_schemas() {
    let root = std::env::temp_dir().join(format!(
        "leadline-snapshot-bad-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("bad.json");
    std::fs::write(&path, r#"{"schema_version": 99, "points": []}"#).unwrap();
    assert!(TrendStore::read(&path).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
