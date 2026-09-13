//! Bounded reads and strict path handling for external analysis inputs.
//!
//! Mutation reports, test maps, and trend stores are attacker-shaped data:
//! they can be huge, deeply nested, or contain paths that try to escape the
//! analysis root. This module owns the one set of limits every adapter uses,
//! so the rules exist once and adapters stay small.

use crate::Result;
use std::path::Path;

/// `leadline.toml` size limit.
pub const CONFIG_BYTES_LIMIT: u64 = 1 << 20;
/// Per-file external report limit.
pub const INPUT_BYTES_LIMIT: u64 = 64 << 20;
/// Aggregate external input limit across one run.
pub const AGGREGATE_BYTES_LIMIT: u64 = 256 << 20;
/// Aggregate normalized row limit across one run.
pub const AGGREGATE_ROWS_LIMIT: u64 = 1_000_000;
/// Maximum JSON nesting depth.
pub const JSON_DEPTH_LIMIT: usize = 128;
/// Maximum XML nesting depth.
pub const XML_DEPTH_LIMIT: usize = 64;

/// Remaining aggregate budget for external inputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputBudget {
    pub bytes_remaining: u64,
    pub rows_remaining: u64,
}

impl Default for InputBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl InputBudget {
    pub fn new() -> Self {
        Self {
            bytes_remaining: AGGREGATE_BYTES_LIMIT,
            rows_remaining: AGGREGATE_ROWS_LIMIT,
        }
    }

    /// Charges `bytes` against the aggregate budget.
    pub fn consume_bytes(&mut self, bytes: u64) -> Result<()> {
        if bytes > self.bytes_remaining {
            return Err("external inputs exceed the aggregate byte budget".into());
        }
        self.bytes_remaining -= bytes;
        Ok(())
    }

    /// Charges `rows` against the aggregate row budget.
    pub fn consume_rows(&mut self, rows: u64) -> Result<()> {
        if rows > self.rows_remaining {
            return Err("external inputs exceed the aggregate row budget".into());
        }
        self.rows_remaining -= rows;
        Ok(())
    }
}

/// Reads a file no larger than `per_file` bytes, charging the aggregate budget.
pub fn read_bounded(path: &Path, per_file: u64, budget: &mut InputBudget) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(format!("input is not a file: {}", path.display()).into());
    }
    if metadata.len() > per_file {
        return Err(format!("input exceeds the per-file limit: {}", path.display()).into());
    }
    budget.consume_bytes(metadata.len())?;
    let bytes = std::fs::read(path)?;
    if bytes.len() as u64 > per_file {
        return Err(format!("input exceeds the per-file limit: {}", path.display()).into());
    }
    Ok(bytes)
}

/// Rejects JSON nesting deeper than [`JSON_DEPTH_LIMIT`].
///
/// Braces inside strings (including escapes) do not count.
pub fn validate_json_depth(bytes: &[u8]) -> Result<()> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > JSON_DEPTH_LIMIT {
                    return Err(format!("JSON nesting exceeds {JSON_DEPTH_LIMIT} levels").into());
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// Validates an XML report before parsing: UTF-8 only, no DTD/DOCTYPE/entity
/// declarations, and nesting no deeper than [`XML_DEPTH_LIMIT`].
pub fn validate_xml_preamble_and_depth(bytes: &[u8]) -> Result<()> {
    std::str::from_utf8(bytes).map_err(|_| "XML input is not valid UTF-8")?;
    if contains_ignore_case(bytes, b"<!doctype") {
        return Err("XML DTD/DOCTYPE declarations are not supported".into());
    }
    if contains_ignore_case(bytes, b"<!entity") {
        return Err("XML entity declarations are not supported".into());
    }
    if let Some(declaration) = xml_declaration(bytes)
        && let Some(encoding) = xml_encoding(declaration)
        && !encoding.eq_ignore_ascii_case("utf-8")
        && !encoding.eq_ignore_ascii_case("utf8")
    {
        return Err(format!("unsupported XML encoding {encoding:?}: expected UTF-8").into());
    }
    validate_xml_depth(bytes, XML_DEPTH_LIMIT)
}

fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len()
        && haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

fn xml_declaration(bytes: &[u8]) -> Option<&[u8]> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace())?;
    let rest = &bytes[start..];
    if !rest.starts_with(b"<?xml") {
        return None;
    }
    let end = rest.windows(2).position(|window| window == b"?>")?;
    Some(&rest[..end])
}

fn xml_encoding(declaration: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(declaration).ok()?;
    let marker = "encoding";
    let at = text.find(marker)? + marker.len();
    let rest = text[at..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

/// Rejects XML nesting deeper than `limit`, skipping comments, CDATA, and
/// processing instructions. Over-counts nothing; only element depth counts.
pub fn validate_xml_depth(bytes: &[u8], limit: usize) -> Result<()> {
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        let Some(offset) = bytes[index..].iter().position(|byte| *byte == b'<') else {
            break;
        };
        let start = index + offset;
        let rest = &bytes[start..];
        if rest.starts_with(b"<!--") {
            let Some(end) = find(rest, b"-->") else {
                return Err("unterminated XML comment".into());
            };
            index = start + end + 3;
            continue;
        }
        if rest.starts_with(b"<![CDATA[") {
            let Some(end) = find(rest, b"]]>") else {
                return Err("unterminated XML CDATA section".into());
            };
            index = start + end + 3;
            continue;
        }
        if rest.starts_with(b"<?") || rest.starts_with(b"<!") {
            let Some(end) = rest.iter().position(|byte| *byte == b'>') else {
                return Err("unterminated XML declaration".into());
            };
            index = start + end + 1;
            continue;
        }
        if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            index = start + 2;
            continue;
        }
        // Opening tag: find the closing `>` outside attribute quotes.
        let mut cursor = start + 1;
        let mut quote: Option<u8> = None;
        while cursor < bytes.len() {
            let byte = bytes[cursor];
            match quote {
                Some(active) if byte == active => quote = None,
                Some(_) => {}
                None if byte == b'"' || byte == b'\'' => quote = Some(byte),
                None if byte == b'>' => break,
                None => {}
            }
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err("unterminated XML tag".into());
        }
        let self_closing = cursor > start && bytes[cursor - 1] == b'/';
        if !self_closing {
            depth += 1;
            if depth > limit {
                return Err(format!("XML nesting exceeds {limit} levels").into());
            }
        }
        index = cursor + 1;
    }
    Ok(())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// A rejected external path, with a stable reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalPathError(String);

impl std::fmt::Display for ExternalPathError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ExternalPathError {}

/// Normalizes a report-supplied relative path, rejecting escapes.
///
/// Rejects absolute paths, drive/UNC prefixes, backslashes, NUL/control
/// characters, empty results, and any `..` that walks above the root.
pub fn strict_relative_path(path: &str) -> std::result::Result<String, ExternalPathError> {
    let reject =
        |reason: &str| ExternalPathError(format!("invalid report path {path:?}: {reason}"));
    if path.is_empty() {
        return Err(reject("empty"));
    }
    if path.chars().any(char::is_control) {
        return Err(reject("control character"));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(reject("absolute path"));
    }
    if path.contains('\\') {
        return Err(reject("backslash separator"));
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(reject("drive prefix"));
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(reject("parent traversal escapes the root"));
                }
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(reject("empty"));
    }
    Ok(parts.join("/"))
}
