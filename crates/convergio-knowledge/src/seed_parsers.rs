//! Content parsers for the knowledge seed pipeline.
//!
//! Extracts structured entries from markdown files:
//! AGENTS.md learnings, ADR decisions,
//! and CONSTITUTION.md rules.

use std::path::Path;

use tracing::warn;

pub type Entry = (String, String); // (content, source_type)

/// AGENTS.md "Key learnings" section.
pub fn parse_learnings(content: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut active = false;
    for line in content.lines() {
        if line.contains("Key learnings") && line.starts_with('#') {
            active = true;
            continue;
        }
        if active && line.starts_with('#') {
            break;
        }
        if active {
            if let Some(t) = line.trim().strip_prefix("- ") {
                if !t.is_empty() {
                    out.push((t.replace("**", ""), "learning".into()));
                }
            }
        }
    }
    out
}

/// ADR decision files from docs/adr/.
pub fn parse_adr_files(root: &Path) -> Vec<Entry> {
    let Ok(dir) = std::fs::read_dir(root.join("docs/adr")) else {
        warn!("seed: docs/adr not found, skipping ADRs");
        return vec![];
    };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("ADR-") || !fname.ends_with(".md") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let title = raw
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").to_string())
            .unwrap_or_else(|| fname.clone());
        let mut secs = String::new();
        let mut cap = false;
        for line in raw.lines() {
            if line.starts_with("## Decision") || line.starts_with("## Consequences") {
                cap = true;
            } else if cap && line.starts_with("## ") {
                cap = false;
            }
            if cap {
                secs.push_str(line);
                secs.push('\n');
            }
        }
        let text = if secs.is_empty() {
            format!("{title}\n\n{}", &raw[..raw.len().min(500)])
        } else {
            format!("{title}\n\n{secs}")
        };
        out.push((text, "decision".into()));
    }
    out
}

/// Numbered sections (phases, sessions, learnings).
pub fn parse_phases(content: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut heading: Option<String> = None;
    let mut buf = String::new();
    let is_section = |line: &str| {
        line.starts_with("## ") && line.len() > 4 && line.as_bytes()[3].is_ascii_digit()
    };
    for line in content.lines() {
        if is_section(line) || (line.starts_with("## ") && heading.is_some()) {
            if let Some(h) = heading.take() {
                if !buf.trim().is_empty() {
                    out.push((format!("{h}\n{}", buf.trim()), "learning".into()));
                }
            }
            buf.clear();
            if is_section(line) {
                heading = Some(line.to_string());
            }
            continue;
        }
        if heading.is_some() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some(h) = heading {
        if !buf.trim().is_empty() {
            out.push((format!("{h}\n{}", buf.trim()), "learning".into()));
        }
    }
    out
}

/// CONSTITUTION.md rules (numbered list items).
pub fn parse_constitution(content: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut rule = String::new();
    for line in content.lines() {
        let t = line.trim();
        let numbered = t.len() > 2 && t.as_bytes()[0].is_ascii_digit() && t.contains(". ");
        if numbered {
            if !rule.is_empty() {
                out.push((rule.clone(), "decision".into()));
            }
            rule = t.to_string();
        } else if !rule.is_empty() && !t.is_empty() && !t.starts_with('#') {
            rule.push(' ');
            rule.push_str(t);
        } else if t.starts_with('#') && !rule.is_empty() {
            out.push((rule.clone(), "decision".into()));
            rule.clear();
        }
    }
    if !rule.is_empty() {
        out.push((rule, "decision".into()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_learnings_extracts_and_strips_bold() {
        let md = "### Key learnings\n- First (#1)\n- **Bold (#2)**\n## Next";
        let items = parse_learnings(md);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].1, "learning");
        assert!(!items[1].0.contains("**"));
    }

    #[test]
    fn parse_phases_extracts() {
        let md = "# Top\n## 1. First\nOne\n## 2. Second\nTwo\n## Other\n";
        let items = parse_phases(md);
        assert_eq!(items.len(), 2);
        assert!(items[0].0.contains("1. First"));
    }

    #[test]
    fn parse_constitution_numbered() {
        let md = "# Title\n1. First rule\n2. Second rule\nwith more\n";
        let items = parse_constitution(md);
        assert_eq!(items.len(), 2);
        assert!(items[1].0.contains("with more"));
    }
}
