use leadline::external::{
    InputBudget, JSON_DEPTH_LIMIT, XML_DEPTH_LIMIT, read_bounded, strict_relative_path,
    validate_json_depth, validate_xml_preamble_and_depth,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-external-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn budget_charges_bytes_and_rows_with_stable_errors() {
    let mut budget = InputBudget {
        bytes_remaining: 10,
        rows_remaining: 3,
    };
    budget.consume_bytes(4).unwrap();
    budget.consume_rows(2).unwrap();
    assert!(budget.consume_bytes(7).is_err());
    assert!(budget.consume_rows(2).is_err());
    assert_eq!(budget.bytes_remaining, 6);
    assert_eq!(budget.rows_remaining, 1);
    assert_eq!(InputBudget::default().bytes_remaining, 256 << 20);
    assert_eq!(InputBudget::default().rows_remaining, 1_000_000);
}

#[test]
fn read_bounded_enforces_per_file_and_aggregate_limits() {
    let root = temporary_directory();
    let file = root.join("report.xml");
    std::fs::write(&file, b"0123456789").unwrap();

    let mut budget = InputBudget {
        bytes_remaining: 20,
        rows_remaining: 1,
    };
    assert_eq!(read_bounded(&file, 10, &mut budget).unwrap(), b"0123456789");
    assert_eq!(budget.bytes_remaining, 10);

    assert!(read_bounded(&file, 9, &mut budget).is_err());
    assert_eq!(budget.bytes_remaining, 10, "rejected reads do not charge");

    let mut small = InputBudget {
        bytes_remaining: 9,
        rows_remaining: 1,
    };
    assert!(read_bounded(&file, 10, &mut small).is_err());
    assert!(read_bounded(&root, 10, &mut small).is_err());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn json_depth_ignores_strings_and_rejects_deep_documents() {
    let shallow = format!("{{\"a\": \"{}[[{{\", \"b\": [[]]}}", "{".repeat(200));
    validate_json_depth(shallow.as_bytes()).unwrap();

    let deep = format!(
        "{}0{}",
        "[".repeat(JSON_DEPTH_LIMIT),
        "]".repeat(JSON_DEPTH_LIMIT)
    );
    validate_json_depth(deep.as_bytes()).unwrap();
    let too_deep = format!(
        "{}0{}",
        "[".repeat(JSON_DEPTH_LIMIT + 1),
        "]".repeat(JSON_DEPTH_LIMIT + 1)
    );
    assert!(validate_json_depth(too_deep.as_bytes()).is_err());

    let escaped = format!("\"{}\"", "\\\"".repeat(10));
    validate_json_depth(escaped.as_bytes()).unwrap();
}

#[test]
fn xml_rejects_doctype_entities_encodings_and_deep_nesting() {
    validate_xml_preamble_and_depth(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><mutations><m/></mutations>",
    )
    .unwrap();
    validate_xml_preamble_and_depth(b"<mutations/>").unwrap();

    for rejected in [
        &b"<?xml version=\"1.0\"?><!DOCTYPE mutations><mutations/>"[..],
        b"<!doctype mutations><mutations/>",
        b"<mutations><!ENTITY x \"y\"><m/></mutations>",
        b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><mutations/>",
    ] {
        assert!(
            validate_xml_preamble_and_depth(rejected).is_err(),
            "accepted: {:?}",
            std::str::from_utf8(rejected)
        );
    }
    assert!(validate_xml_preamble_and_depth(&[0xff, 0xfe]).is_err());

    let deep = format!(
        "{}<m/>{}",
        "<m>".repeat(XML_DEPTH_LIMIT),
        "</m>".repeat(XML_DEPTH_LIMIT)
    );
    validate_xml_preamble_and_depth(deep.as_bytes()).unwrap();
    let too_deep = format!(
        "{}<m/>{}",
        "<m>".repeat(XML_DEPTH_LIMIT + 1),
        "</m>".repeat(XML_DEPTH_LIMIT + 1)
    );
    assert!(validate_xml_preamble_and_depth(too_deep.as_bytes()).is_err());

    // Comments, CDATA, and attribute text never change element depth. The
    // DOCTYPE scan stays deliberately conservative: declaration-like text
    // anywhere in the bytes is rejected before quick-xml sees it.
    validate_xml_preamble_and_depth(b"<!-- <fake> --><m a=\"<x>\"><![CDATA[<not-a-tag>]]></m>")
        .unwrap();
}

#[test]
fn strict_paths_reject_escapes_and_normalize_safe_forms() {
    assert_eq!(strict_relative_path("src/a.ts").unwrap(), "src/a.ts");
    assert_eq!(strict_relative_path("./src//a.ts").unwrap(), "src/a.ts");
    assert_eq!(strict_relative_path("src/./a.ts").unwrap(), "src/a.ts");
    assert_eq!(strict_relative_path("src/x/../a.ts").unwrap(), "src/a.ts");

    for rejected in [
        "",
        "/src/a.ts",
        "\\src\\a.ts",
        "C:\\src\\a.ts",
        "c:src/a.ts",
        "../src/a.ts",
        "src/../../a.ts",
        "..",
        "a\0b.ts",
        "a\nb.ts",
    ] {
        assert!(
            strict_relative_path(rejected).is_err(),
            "accepted: {rejected:?}"
        );
    }
}
