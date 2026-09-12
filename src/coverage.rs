use crate::Result;
use crate::core::{FileAnalysis, FunctionAnalysis, apply_coverage};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageMap {
    files: BTreeMap<String, BTreeMap<u32, u64>>,
}

impl CoverageMap {
    pub fn from_lcov(text: &str) -> Result<Self> {
        let mut coverage = Self::default();
        let mut current_file = None;
        for line in text.lines() {
            if let Some(path) = line.strip_prefix("SF:") {
                current_file = Some(normalize_coverage_path(path.trim()));
            } else if let Some(data) = line.strip_prefix("DA:") {
                let Some(path) = current_file.as_ref() else {
                    continue;
                };
                let mut fields = data.split(',');
                let number: u32 = fields
                    .next()
                    .ok_or("LCOV DA record has no line number")?
                    .parse()?;
                let count: u64 = fields
                    .next()
                    .ok_or("LCOV DA record has no execution count")?
                    .parse()?;
                coverage.insert(path.clone(), number, count);
            } else if line == "end_of_record" {
                current_file = None;
            }
        }
        Ok(coverage)
    }

    pub fn from_jacoco_xml(text: &str) -> Result<Self> {
        let mut coverage = Self::default();
        let mut reader = Reader::from_str(text);
        reader.config_mut().trim_text(true);
        let mut package = String::new();
        let mut source_file = None;

        loop {
            match reader.read_event()? {
                Event::Start(element) => match element.name().as_ref() {
                    "package" => {
                        package = attribute(&element, "name")?.unwrap_or_default();
                    }
                    "sourcefile" => {
                        source_file = attribute(&element, "name")?.map(|name| {
                            let path = if package.is_empty() {
                                name
                            } else {
                                format!("{package}/{name}")
                            };
                            normalize_coverage_path(&path)
                        });
                    }
                    _ => {}
                },
                Event::Empty(element) if element.name().as_ref() == "line" => {
                    let Some(path) = source_file.as_ref() else {
                        continue;
                    };
                    let number: u32 = attribute(&element, "nr")?
                        .ok_or("JaCoCo line has no nr attribute")?
                        .parse()?;
                    let covered: u64 = attribute(&element, "ci")?
                        .ok_or("JaCoCo line has no ci attribute")?
                        .parse()?;
                    coverage.insert(path.clone(), number, covered);
                }
                Event::End(element) if element.name().as_ref() == "sourcefile" => {
                    source_file = None;
                }
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(coverage)
    }

    pub fn merge(&mut self, other: Self) {
        for (path, lines) in other.files {
            let target = self.files.entry(path).or_default();
            for (line, count) in lines {
                target
                    .entry(line)
                    .and_modify(|existing| *existing = existing.saturating_add(count))
                    .or_insert(count);
            }
        }
    }

    pub fn apply(&self, file: &mut FileAnalysis) {
        let lines = self.lines_for_path(&file.path);
        for function in &mut file.functions {
            let ratio = lines.and_then(|lines| {
                let relevant = lines.range(function.start_line..=function.end_line);
                let mut total = 0_u32;
                let mut covered = 0_u32;
                for (_, count) in relevant {
                    total += 1;
                    covered += u32::from(*count > 0);
                }
                (total > 0).then(|| f64::from(covered) / f64::from(total))
            });
            apply_coverage(function, ratio);
        }
    }
    /// Execution hits for one exact source line: `Some(count)` when the line
    /// is known, `None` when no coverage record covers the path/line.
    /// Line coverage only; branch data is never consulted.
    pub fn hits(&self, path: &str, line: u32) -> Option<u64> {
        self.lines_for_path(path)?.get(&line).copied()
    }

    /// Apply line coverage to a single function by path. Functions in files
    /// with no supported language stay untouched.
    pub fn apply_function(&self, path: &str, function: &mut FunctionAnalysis) {
        let Some(language) = crate::parser::detect_language(path) else {
            return;
        };
        let mut file = FileAnalysis {
            path: path.to_owned(),
            language,
            functions: vec![function.clone()],
            parse_errors: Vec::new(),
        };
        self.apply(&mut file);
        if let Some(updated) = file.functions.pop() {
            *function = updated;
        }
    }

    fn insert(&mut self, path: String, line: u32, count: u64) {
        self.files
            .entry(path)
            .or_default()
            .entry(line)
            .and_modify(|existing| *existing = existing.saturating_add(count))
            .or_insert(count);
    }

    fn lines_for_path(&self, path: &str) -> Option<&BTreeMap<u32, u64>> {
        if let Some(lines) = self.files.get(path) {
            return Some(lines);
        }
        let candidate_suffix = format!("/{path}");
        let mut matches = self.files.iter().filter(|(candidate, _)| {
            candidate.ends_with(&candidate_suffix) || path.ends_with(&format!("/{candidate}"))
        });
        let (_, lines) = matches.next()?;
        matches.next().is_none().then_some(lines)
    }
}

fn normalize_coverage_path(value: &str) -> String {
    let replaced = value.replace('\\', "/");
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn attribute(element: &BytesStart<'_>, key: &str) -> Result<Option<String>> {
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute?;
        if attribute.key.as_ref() == key {
            return Ok(Some(
                attribute
                    .normalized_value(XmlVersion::Implicit1_0)?
                    .into_owned(),
            ));
        }
    }
    Ok(None)
}
