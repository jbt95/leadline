use leadline::index::{AnalysisIndex, IndexedFile, content_key};

mod common;
use common::temporary_directory;

#[test]
fn content_key_is_content_sensitive() {
    assert_ne!(content_key(b"let a = 1;"), content_key(b"let a = 2;"));
}

#[test]
fn index_is_unusable_after_config_or_scope_change() {
    let index = AnalysisIndex::empty(".", "cfg-a");
    assert!(index.is_usable(".", "cfg-a"));
    assert!(!index.is_usable(".", "cfg-b"));
    assert!(!index.is_usable("src", "cfg-a"));
}

#[test]
fn missing_and_corrupt_indexes_open_empty() {
    let missing = temporary_directory();
    assert!(AnalysisIndex::open(&missing).files.is_empty());

    let corrupt = temporary_directory();
    std::fs::write(corrupt.join("index.json"), b"{ not json").unwrap();
    assert!(AnalysisIndex::open(&corrupt).files.is_empty());
}

#[test]
fn index_round_trips_and_saves_deterministically() {
    let dir = temporary_directory();
    let mut index = AnalysisIndex::empty(".", "cfg");
    index.files.insert(
        "b.ts".to_owned(),
        IndexedFile {
            size: 3,
            key: content_key(b"bbb"),
            functions: Vec::new(),
        },
    );
    index.files.insert(
        "a.ts".to_owned(),
        IndexedFile {
            size: 3,
            key: content_key(b"aaa"),
            functions: Vec::new(),
        },
    );
    index.save(&dir).unwrap();
    let first = std::fs::read(dir.join("index.json")).unwrap();
    index.save(&dir).unwrap();
    let second = std::fs::read(dir.join("index.json")).unwrap();
    assert_eq!(first, second, "index JSON must be deterministic");

    let reopened = AnalysisIndex::open(&dir);
    assert_eq!(
        reopened.files.keys().collect::<Vec<_>>(),
        vec!["a.ts", "b.ts"]
    );
    assert!(reopened.is_usable(".", "cfg"));
}
