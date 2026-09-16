use leadline::index::{Reuse, load_entries, refresh_files, scope_label};
use std::path::{Path, PathBuf};

mod common;
use common::temporary_directory;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/complex-project/repo/current")
}

#[test]
fn first_refresh_analyzes_everything() {
    let root = fixture();
    let entries = load_entries(&root, &[]).unwrap();
    let (report, index, reuse) = refresh_files(None, ".", "cfg", &entries).unwrap();
    assert_eq!(
        reuse,
        Reuse {
            analyzed: entries.len(),
            reused: 0
        }
    );
    assert_eq!(index.files.len(), entries.len());
    assert_eq!(report.files.len(), entries.len());
}

#[test]
fn second_refresh_reuses_everything_and_matches_cold_output() {
    let root = fixture();
    let entries = load_entries(&root, &[]).unwrap();
    let (cold, index, _) = refresh_files(None, ".", "cfg", &entries).unwrap();
    let (warm, rebuilt, reuse) = refresh_files(Some(&index), ".", "cfg", &entries).unwrap();
    assert_eq!(
        reuse,
        Reuse {
            analyzed: 0,
            reused: entries.len()
        }
    );
    assert_eq!(
        serde_json::to_vec(&cold).unwrap(),
        serde_json::to_vec(&warm).unwrap(),
        "warm output must be byte-identical to cold output"
    );
    assert_eq!(index.files, rebuilt.files);
}

#[test]
fn changed_file_is_reanalyzed_and_others_are_reused() {
    let root = fixture();
    let entries = load_entries(&root, &[]).unwrap();
    let (_, index, _) = refresh_files(None, ".", "cfg", &entries).unwrap();

    let mut edited = entries.clone();
    let target = edited.first_mut().unwrap();
    target.bytes.extend_from_slice(b"\n// edited\n");

    let (_, _, reuse) = refresh_files(Some(&index), ".", "cfg", &edited).unwrap();
    assert_eq!(reuse.analyzed, 1);
    assert_eq!(reuse.reused, entries.len() - 1);
}

#[test]
fn stale_config_or_scope_forces_full_reanalysis() {
    let root = fixture();
    let entries = load_entries(&root, &[]).unwrap();
    let (_, index, _) = refresh_files(None, ".", "cfg", &entries).unwrap();

    let (_, _, wrong_config) = refresh_files(Some(&index), ".", "other", &entries).unwrap();
    assert_eq!(wrong_config.reused, 0);
    let (_, _, wrong_scope) = refresh_files(Some(&index), "src", "cfg", &entries).unwrap();
    assert_eq!(wrong_scope.reused, 0);
}

#[test]
fn scope_label_of_the_current_directory_is_dot() {
    assert_eq!(scope_label(Path::new(".")), ".");
}

#[test]
fn parse_error_files_are_never_stored_and_are_always_reanalyzed() {
    let entries = vec![leadline::source_snapshot::SourceEntry {
        path: "broken.ts".to_owned(),
        bytes: b"function ( {".to_vec(),
    }];
    let (report, index, reuse) = refresh_files(None, ".", "cfg", &entries).unwrap();
    assert!(!index.files.contains_key("broken.ts"));
    assert_eq!(report.files.len(), 1);
    assert!(!report.files[0].parse_errors.is_empty());
    assert_eq!(reuse.analyzed, 1);

    let (_, index, reuse) = refresh_files(Some(&index), ".", "cfg", &entries).unwrap();
    assert!(!index.files.contains_key("broken.ts"));
    assert_eq!(reuse.analyzed, 1);
    assert_eq!(reuse.reused, 0);
}

#[test]
fn warm_refresh_preserves_stored_history_when_usable() {
    let root = fixture();
    let entries = load_entries(&root, &[]).unwrap();
    let (_, mut index, _) = refresh_files(None, ".", "cfg", &entries).unwrap();
    index.history = Some(leadline::index::HistoryFacts {
        head_commit: Some("abc".to_owned()),
        head_timestamp: Some(1),
        files: Vec::new(),
    });

    let (_, preserved, _) = refresh_files(Some(&index), ".", "cfg", &entries).unwrap();
    assert_eq!(preserved.history, index.history);

    let (_, wrong_scope, _) = refresh_files(Some(&index), "other", "cfg", &entries).unwrap();
    assert!(wrong_scope.history.is_none());

    let (_, wrong_config, _) = refresh_files(Some(&index), ".", "other", &entries).unwrap();
    assert!(wrong_config.history.is_none());
}

#[test]
fn history_is_unavailable_outside_a_repository() {
    let dir = temporary_directory();
    assert!(leadline::index::refresh_history(None, &dir).is_none());
}
