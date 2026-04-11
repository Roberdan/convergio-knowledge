//! Baseline seed — ingest ALL existing knowledge into the vector store.
//!
//! Sources: AGENTS.md learnings, ADR decisions,
//! knowledge_base table, completed plans, and CONSTITUTION.md rules.
//! Idempotent: uses content-hash source_ids to skip duplicates.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use convergio_db::pool::{ConnPool, PooledConn};
use tracing::{info, warn};

use crate::seed_parsers::{parse_adr_files, parse_constitution, parse_learnings};
use crate::store::LanceVectorStore;

#[derive(Debug, Clone)]
pub struct SeedReport {
    pub total: usize,
    pub by_source: Vec<(String, usize)>,
    pub skipped: usize,
    pub errors: Vec<String>,
}

type Entry = (String, String);

fn content_hash(s: &str) -> String {
    let h = s
        .bytes()
        .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    format!("seed-{h:016x}")
}

fn repo_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CONVERGIO_REPO_ROOT") {
        return Some(PathBuf::from(p));
    }
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
}

fn read_file(root: &std::path::Path, rel: &str) -> Option<String> {
    let path = root.join(rel);
    std::fs::read_to_string(&path)
        .inspect_err(|e| warn!(path = %path.display(), error = %e, "seed: skipping"))
        .ok()
}

fn has_table(pool: &ConnPool, name: &str) -> Option<PooledConn> {
    let conn = pool.get().ok()?;
    let sql = format!("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='{name}'");
    let exists: bool = conn
        .query_row(&sql, [], |r| r.get::<_, i64>(0).map(|c| c > 0))
        .unwrap_or(false);
    if exists {
        Some(conn)
    } else {
        None
    }
}

fn seed_from_kb(pool: &ConnPool) -> Vec<Entry> {
    let Some(conn) = has_table(pool, "knowledge_base") else {
        info!("seed: knowledge_base table not found, skipping");
        return vec![];
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT domain, title, content FROM knowledge_base ORDER BY title")
    else {
        return vec![];
    };
    stmt.query_map([], |r| {
        let (d, t, c): (String, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
        Ok((
            format!(
                "[{d}] {t}

{c}"
            ),
            "kb".to_string(),
        ))
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

fn seed_from_plans(pool: &ConnPool) -> Vec<Entry> {
    let Some(conn) = has_table(pool, "plans") else {
        return vec![];
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT id, name, objective FROM plans WHERE status='done' ORDER BY id")
    else {
        return vec![];
    };
    stmt.query_map([], |r| {
        let (id, n, o): (i64, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
        Ok((
            format!(
                "Plan #{id}: {n}

{o}"
            ),
            "task".to_string(),
        ))
    })
    .map(|rows| rows.filter_map(|r| r.ok()).collect())
    .unwrap_or_default()
}

/// Ingest all known sources into the vector store. Idempotent.
pub async fn seed_baseline(store: &LanceVectorStore, pool: &ConnPool) -> SeedReport {
    let mut report = SeedReport {
        total: 0,
        by_source: Vec::new(),
        skipped: 0,
        errors: Vec::new(),
    };
    let existing: HashSet<String> = store.all_source_ids().await.unwrap_or_default();
    info!(
        existing = existing.len(),
        "seed: loaded existing source_ids"
    );

    let root = match repo_root() {
        Some(r) => r,
        None => {
            report.errors.push("cannot determine repo root".into());
            return report;
        }
    };

    let mut all: Vec<(String, String, &str)> = Vec::new();
    macro_rules! collect {
        ($items:expr, $label:expr) => {{
            let items = $items;
            info!(count = items.len(), concat!("seed: ", $label));
            all.extend(items.into_iter().map(|(c, t)| (c, t, $label)));
        }};
    }
    if let Some(text) = read_file(&root, "AGENTS.md") {
        collect!(parse_learnings(&text), "agents-learnings");
    }
    collect!(parse_adr_files(&root), "adr");
    collect!(seed_from_kb(pool), "knowledge-base");
    collect!(seed_from_plans(pool), "plans");
    if let Some(text) = read_file(&root, "CONSTITUTION.md") {
        collect!(parse_constitution(&text), "constitution");
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (content, source_type, label) in &all {
        let sid = content_hash(content);
        if existing.contains(&sid) {
            report.skipped += 1;
            continue;
        }
        let emb = crate::embedder::embed(content).await;
        match store
            .insert(content, source_type, &sid, None, None, None, "org", &emb)
            .await
        {
            Ok(_) => {
                *counts.entry(label).or_insert(0) += 1;
                report.total += 1;
            }
            Err(e) => report.errors.push(format!("[{label}] {e}")),
        }
    }
    report.by_source = counts
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    report.by_source.sort_by(|a, b| a.0.cmp(&b.0));
    info!(
        total = report.total,
        skipped = report.skipped,
        "seed_baseline complete"
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_deterministic_and_differs() {
        assert_eq!(content_hash("hello"), content_hash("hello"));
        assert!(content_hash("hello").starts_with("seed-"));
        assert_ne!(content_hash("hello"), content_hash("world"));
    }
}
