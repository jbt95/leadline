//! Deterministic seeded-mutation robustness corpus.
//!
//! Fixed seed (`CORPUS_SEED`) so every failure reproduces byte-for-byte.
//! No new dependencies, no libfuzzer: a small xorshift64 PRNG drives all
//! mutations.
//!
//! Partial-result semantics under test: broken input never fails the whole
//! analysis. [`leadline::analyze_source`] always returns `Ok` for a supported
//! extension; syntax breakage surfaces inline as `FileAnalysis::parse_errors`
//! while any recoverable functions are still reported. Coverage parsers
//! (`CoverageMap::from_lcov` / `from_jacoco_xml`) may return `Ok` or `Err`,
//! but must never panic, and applying parsed coverage to a file must never
//! panic either.

use leadline::coverage::CoverageMap;

/// Fixed seed: every run mutates identically, so failures reproduce.
const CORPUS_SEED: u64 = 0x51_7E_2B_9C_3D_4F_11_A5;

/// Minimum iteration budgets enforced by assertions below.
const PARSER_ITERATIONS: usize = 240;
const COVERAGE_ITERATIONS_PER_FORMAT: usize = 60;

/// Tiny deterministic xorshift64* PRNG (no external deps).
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        // Zero is a fixed point for xorshift; remap it.
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
}

const SOURCE_SEEDS: &[(&str, &str)] = &[
    (
        "seed.ts",
        "function add(a: number, b: number): number {\n  if (b > 0) {\n    return a + b;\n  }\n  return a;\n}\n",
    ),
    (
        "seed.js",
        "function mul(x, y) {\n  let total = 0;\n  for (let i = 0; i < y; i++) {\n    total += x;\n  }\n  return total;\n}\n",
    ),
    (
        "seed.tsx",
        "function Greet(props: { name: string }) {\n  if (!props.name) {\n    return <span>hi</span>;\n  }\n  return <h1>Hello {props.name}</h1>;\n}\n",
    ),
    (
        "Seed.java",
        "class Seed {\n  int max(int a, int b) {\n    if (a > b) {\n      return a;\n    }\n    return b;\n  }\n}\n",
    ),
    (
        "seed.c",
        "int max(int a, int b) {\n  if (a > b) {\n    return a;\n  }\n  return b;\n}\n",
    ),
    (
        "seed.rs",
        "struct Counter {\n    n: i32,\n}\n\nimpl Counter {\n    fn add(&mut self, delta: i32) -> Option<i32> {\n        match delta {\n            0 => None,\n            _ => {\n                self.n += delta;\n                Some(self.n)\n            }\n        }\n    }\n}\n\nfn first(values: &[i32]) -> Option<i32> {\n    let mut found = None;\n    'outer: for value in values {\n        if *value > 0 {\n            found = Some(*value);\n            break 'outer;\n        }\n    }\n    found\n}\n",
    ),
];

const LCOV_SEED: &str = "TN:\nSF:seed.ts\nDA:1,1\nDA:2,1\nDA:3,0\nDA:5,2\nend_of_record\nSF:Seed.java\nDA:2,3\nDA:5,0\nend_of_record\n";

const JACOCO_SEED: &str = r#"<report name="seed"><package name="com/example"><sourcefile name="Seed.java"><line nr="2" mi="0" ci="3"/><line nr="5" mi="1" ci="0"/></sourcefile></package><package name=""><sourcefile name="seed.ts"><line nr="2" mi="0" ci="1"/><line nr="3" mi="1" ci="0"/></sourcefile></package></report>"#;

const CONFLICT_MARKERS: &[&[u8]] = &[b"<<<<<<< HEAD\n", b"=======\n", b">>>>>>> feature-branch\n"];

/// Unusual but valid UTF-8: emoji, ZWJ sequences, RTL override, CJK, zero-width.
const WEIRD_UNICODE: &[&str] = &[
    "\u{1F600}\u{1F525}\u{1F4A9}",
    "\u{200D}\u{200B}\u{FEFF}",
    "\u{202E}reversed\u{202C}",
    "\u{4E2D}\u{6587}\u{D55C}\u{AE00}",
    "Z\u{0308}a\u{0301}l\u{0327}g\u{0303}o\u{0336}",
];

/// Mutate a source seed deterministically. `op` cycles through every
/// mutation family so each family is exercised regardless of PRNG drift.
fn mutate_source(rng: &mut XorShift64, seed: &[u8], op: usize) -> Vec<u8> {
    let mut buf = seed.to_vec();
    match op % 8 {
        // 0: random byte flips.
        0 => {
            let flips = 1 + rng.below(8);
            for _ in 0..flips {
                if buf.is_empty() {
                    buf.push(rng.next_u64() as u8);
                } else {
                    let at = rng.below(buf.len());
                    buf[at] = rng.next_u64() as u8;
                }
            }
        }
        // 1: truncation to a random prefix (possibly empty).
        1 => {
            let at = rng.below(buf.len().saturating_add(1));
            buf.truncate(at);
        }
        // 2: splice in merge-conflict markers.
        2 => {
            let marker = CONFLICT_MARKERS[rng.below(CONFLICT_MARKERS.len())];
            let at = rng.below(buf.len().saturating_add(1));
            buf.splice(at..at, marker.iter().copied());
        }
        // 3: inject unusual Unicode.
        3 => {
            let sample = WEIRD_UNICODE[rng.below(WEIRD_UNICODE.len())];
            let at = rng.below(buf.len().saturating_add(1));
            buf.splice(at..at, sample.as_bytes().iter().copied());
        }
        // 4: overlong line injection (10k-40k bytes).
        4 => {
            let len = 10_000 + rng.below(30_001);
            let fill = b"x/*a"[rng.below(4)];
            let at = rng.below(buf.len().saturating_add(1));
            buf.splice(at..at, std::iter::repeat_n(fill, len));
        }
        // 5: invalid UTF-8 byte sequences.
        5 => {
            const BAD: &[&[u8]] = &[
                &[0xFF],
                &[0xFE],
                &[0x80],
                &[0xC0, 0xAF],
                &[0xED, 0xA0, 0x80],
                &[0xF0, 0x28, 0x8C, 0x28],
            ];
            let bad = BAD[rng.below(BAD.len())];
            if buf.is_empty() {
                buf.extend_from_slice(bad);
            } else {
                let at = rng.below(buf.len());
                let end = (at + rng.below(4)).min(buf.len());
                buf.splice(at..end, bad.iter().copied());
            }
        }
        // 6: duplicate a random slice at a random position.
        6 => {
            if !buf.is_empty() {
                let from = rng.below(buf.len());
                let len = 1 + rng.below(buf.len() - from);
                let chunk = buf[from..from + len].to_vec();
                let at = rng.below(buf.len().saturating_add(1));
                buf.splice(at..at, chunk);
            }
        }
        // 7: delete a random span (possibly the whole input).
        _ => {
            if !buf.is_empty() {
                let from = rng.below(buf.len());
                let len = rng.below(buf.len() - from + 1);
                buf.drain(from..from + len);
            }
        }
    }
    buf
}

/// Mutate a coverage seed deterministically (string level; byte-level
/// hostility for coverage comes from the lossy invalid-UTF-8 case below).
fn mutate_coverage(rng: &mut XorShift64, seed: &str, op: usize) -> String {
    let mut text = seed.to_string();
    match op % 7 {
        // 0: random char replacement with hostile alphabet.
        0 => {
            const HOSTILE: &[char] = &[
                '\0',
                '\n',
                ',',
                ':',
                '<',
                '>',
                '"',
                '\'',
                '&',
                '-',
                'e',
                '9',
                '\u{1F600}',
                '\u{202E}',
            ];
            let mut chars: Vec<char> = text.chars().collect();
            let flips = 1 + rng.below(6);
            for _ in 0..flips {
                if chars.is_empty() {
                    chars.push(HOSTILE[rng.below(HOSTILE.len())]);
                } else {
                    let at = rng.below(chars.len());
                    chars[at] = HOSTILE[rng.below(HOSTILE.len())];
                }
            }
            text = chars.into_iter().collect();
        }
        // 1: truncation.
        1 => {
            let chars: Vec<char> = text.chars().collect();
            let at = rng.below(chars.len().saturating_add(1));
            text = chars[..at].iter().collect();
        }
        // 2: duplicate a random line.
        2 => {
            let lines: Vec<&str> = text.lines().collect();
            if !lines.is_empty() {
                let line = lines[rng.below(lines.len())].to_owned();
                text.push('\n');
                text.push_str(&line);
            }
        }
        // 3: inject merge-conflict marker lines.
        3 => {
            const MARKERS: &[&str] = &["<<<<<<< HEAD", "=======", ">>>>>>> feature-branch"];
            text.push('\n');
            text.push_str(MARKERS[rng.below(MARKERS.len())]);
            text.push('\n');
        }
        // 4: hostile counters / attributes.
        4 => {
            const COUNTS: &[&str] = &[
                "99999999999999999999",
                "-1",
                "0",
                "1.5",
                "NaN",
                "",
                "18446744073709551615",
            ];
            let count = COUNTS[rng.below(COUNTS.len())];
            text = text.replace("1", count);
        }
        // 5: NUL bytes and broken structure.
        5 => {
            text.push('\0');
            text.push_str("<unclosed");
        }
        // 6: swap in absurd line numbers.
        _ => {
            text = text.replace("2", "4294967295");
        }
    }
    text
}

#[test]
fn seeded_mutation_corpus_never_panics_and_stays_partial() {
    let mut rng = XorShift64::new(CORPUS_SEED);

    // Parser corpus: every input must yield Ok; breakage is inline
    // parse_errors, never Err. A panic fails the test outright.
    let mut parser_runs = 0_usize;
    for i in 0..PARSER_ITERATIONS {
        let (path, seed) = SOURCE_SEEDS[i % SOURCE_SEEDS.len()];
        let mutated = mutate_source(&mut rng, seed.as_bytes(), i);
        let result = leadline::analyze_source(path, &mutated);
        assert!(
            result.is_ok(),
            "analyze_source must return Ok for mutated input {i} ({path})"
        );
        parser_runs += 1;
    }
    assert!(
        parser_runs >= 200,
        "parser corpus ran {parser_runs} iterations"
    );

    // Coverage corpus: parsers may return Ok or Err, but never panic;
    // applying a successfully parsed map must never panic either.
    let mut coverage_runs = 0_usize;
    for i in 0..COVERAGE_ITERATIONS_PER_FORMAT {
        let mutated = mutate_coverage(&mut rng, LCOV_SEED, i);
        let parsed = CoverageMap::from_lcov(&mutated);
        if let Ok(map) = parsed {
            let mut file = leadline::analyze_source("seed.ts", SOURCE_SEEDS[0].1.as_bytes())
                .expect("valid seed analyzes");
            map.apply(&mut file);
        }
        coverage_runs += 1;

        let mutated = mutate_coverage(&mut rng, JACOCO_SEED, i);
        let parsed = CoverageMap::from_jacoco_xml(&mutated);
        if let Ok(map) = parsed {
            let mut file = leadline::analyze_source("Seed.java", SOURCE_SEEDS[3].1.as_bytes())
                .expect("valid seed analyzes");
            map.apply(&mut file);
        }
        coverage_runs += 1;
    }
    // Invalid-UTF-8 bytes through the lossy path must not panic either.
    for i in 0..8 {
        let (_, seed) = SOURCE_SEEDS[i % SOURCE_SEEDS.len()];
        let mutated = mutate_source(&mut rng, seed.as_bytes(), 5);
        let lossy = String::from_utf8_lossy(&mutated).into_owned();
        let lcov = format!("SF:fuzz.ts\nDA:1,1\n{lossy}\nend_of_record\n");
        if let Ok(map) = CoverageMap::from_lcov(&lcov) {
            let mut file = leadline::analyze_source("fuzz.ts", b"function f() { return 1; }\n")
                .expect("valid seed analyzes");
            map.apply(&mut file);
        }
        if let Ok(map) = CoverageMap::from_jacoco_xml(&lossy) {
            let mut file = leadline::analyze_source("fuzz.ts", b"function f() { return 1; }\n")
                .expect("valid seed analyzes");
            map.apply(&mut file);
        }
        coverage_runs += 2;
    }
    assert!(
        coverage_runs >= 100,
        "coverage corpus ran {coverage_runs} iterations"
    );
}

#[test]
fn broken_file_does_not_affect_neighbor() {
    // A single broken file must not change analysis of a valid neighbor:
    // analyze both, and the valid one keeps its exact expected metrics.
    let valid_path = "neighbor.ts";
    let valid_source = b"function add(a: number, b: number): number {\n  if (b > 0) {\n    return a + b;\n  }\n  return a;\n}\n";
    let baseline = leadline::analyze_source(valid_path, valid_source).expect("valid seed analyzes");
    assert!(baseline.parse_errors.is_empty());
    assert_eq!(baseline.functions.len(), 1);

    let broken = leadline::analyze_source(
        "broken.ts",
        b"function broken( {\n<<<<<<< HEAD\n=======\n>>>>>>> branch\n",
    )
    .expect("broken input still returns Ok with inline parse_errors");

    let rerun = leadline::analyze_source(valid_path, valid_source).expect("valid seed analyzes");
    assert_eq!(
        rerun, baseline,
        "neighbor analysis must be bit-identical after a broken file"
    );
    assert_eq!(rerun.functions[0].name, "add");
    assert!(!broken.functions.is_empty() || !broken.parse_errors.is_empty());
}

#[test]
fn hostile_single_inputs_never_panic() {
    // Overlong single line (200k chars, no newline).
    let long_line = vec![b'a'; 200_000];
    let file = leadline::analyze_source("long.js", &long_line).expect("Ok with parse_errors");
    assert_eq!(file.path, "long.js");

    // 1MB+ valid-shaped input: repeated simple functions.
    let mut big = Vec::new();
    for i in 0..40_000 {
        big.extend_from_slice(format!("function f{i}() {{ return {i}; }}\n").as_bytes());
    }
    assert!(big.len() > 1_000_000, "big input exceeds 1MB");
    let file = leadline::analyze_source("big.js", &big).expect("Ok with parse_errors");
    assert_eq!(file.functions.len(), 40_000);

    // Minified single-line JS.
    let minified = b"function a(x){if(x){return 1}return 0}function b(y){return y?1:0}";
    let file = leadline::analyze_source("min.js", minified).expect("Ok with parse_errors");
    assert_eq!(file.functions.len(), 2);

    // Merge-conflict markers inside otherwise valid code.
    let conflict = b"function ok() { return 1; }\n<<<<<<< HEAD\nfunction a() {}\n=======\nfunction b() {}\n>>>>>>> branch\n";
    leadline::analyze_source("conflict.ts", conflict).expect("Ok with parse_errors");

    // Unusual Unicode identifiers and comments.
    let unicode = "function \u{4E2D}\u{6587}() { return 1; } // \u{1F600}\u{200D}\u{202E}comment\n"
        .as_bytes();
    leadline::analyze_source("unicode.ts", unicode).expect("Ok with parse_errors");

    // Invalid UTF-8 byte sequences straight to the parser.
    let invalid: &[u8] = b"function f() {\n\xFF\xFE\x80\xC0\xAF return 1;\n}\n";
    leadline::analyze_source("invalid.js", invalid).expect("Ok with parse_errors");

    // Empty and NUL-only inputs.
    leadline::analyze_source("empty.ts", b"").expect("Ok with parse_errors");
    leadline::analyze_source("nul.js", b"\0\0\0\0").expect("Ok with parse_errors");
}
