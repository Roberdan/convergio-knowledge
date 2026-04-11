//! Auto-pruning + dedup for the knowledge vector store (#688).
//!
//! Strategies:
//! - **Stale pruning**: remove entries older than a threshold by source_type
//! - **Content dedup**: merge entries with near-identical content (prefix match)
//! - **Source dedup**: if same source_type+source_id has multiple entries, keep newest

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::store::LanceVectorStore;
use crate::types::KnowledgeEntry;

/// Default max age in days per source_type before pruning.
fn default_max_age_days(source_type: &str) -> Option<i64> {
    match source_type {
        "task" => Some(30),
        "commit" => Some(60),
        "agent_memory" => Some(14),
        "doc" | "kb" => None, // evergreen, never auto-prune
        _ => Some(30),
    }
}

/// Prune report returned by the pruning endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct PruneReport {
    pub stale_removed: usize,
    pub duplicates_removed: usize,
    pub source_dedup_removed: usize,
    pub total_removed: usize,
    pub total_before: usize,
    pub total_after: usize,
    pub errors: Vec<String>,
}

/// Request for the prune endpoint.
#[derive(Debug, Deserialize)]
pub struct PruneRequest {
    /// Override max age in days (applies to all source_types).
    #[serde(default)]
    pub max_age_days: Option<i64>,
    /// Only prune entries matching this source_type.
    #[serde(default)]
    pub source_type: Option<String>,
    /// Dry run — report what would be pruned without deleting.
    #[serde(default)]
    pub dry_run: bool,
}

/// Run the full prune pipeline.
pub async fn prune(store: &Arc<LanceVectorStore>, req: &PruneRequest) -> PruneReport {
    let mut report = PruneReport {
        stale_removed: 0,
        duplicates_removed: 0,
        source_dedup_removed: 0,
        total_removed: 0,
        total_before: 0,
        total_after: 0,
        errors: vec![],
    };

    let entries = match store.list_all_metadata().await {
        Ok(e) => e,
        Err(e) => {
            report.errors.push(format!("list failed: {e}"));
            return report;
        }
    };
    report.total_before = entries.len();

    let now = chrono::Utc::now();
    let mut to_delete: Vec<String> = Vec::new();

    // 1. Stale pruning — remove entries older than max_age_days
    let stale_ids = find_stale(&entries, &now, req);
    report.stale_removed = stale_ids.len();
    to_delete.extend(stale_ids);

    // 2. Content dedup — entries with identical first 200 chars, keep newest
    let dedup_ids = find_content_duplicates(&entries, &to_delete);
    report.duplicates_removed = dedup_ids.len();
    to_delete.extend(dedup_ids);

    // 3. Source dedup — same source_type+source_id, keep newest
    let source_dedup_ids = find_source_duplicates(&entries, &to_delete);
    report.source_dedup_removed = source_dedup_ids.len();
    to_delete.extend(source_dedup_ids);

    report.total_removed = to_delete.len();
    report.total_after = report.total_before.saturating_sub(report.total_removed);

    if req.dry_run || to_delete.is_empty() {
        return report;
    }

    // Batch delete in chunks of 50
    for chunk in to_delete.chunks(50) {
        let ids: Vec<String> = chunk.to_vec();
        match store.delete_batch(&ids).await {
            Ok(n) => info!(deleted = n, "pruned knowledge entries"),
            Err(e) => {
                warn!(error = %e, "prune batch delete failed");
                report.errors.push(e);
            }
        }
    }
    report
}

fn find_stale(
    entries: &[KnowledgeEntry],
    now: &chrono::DateTime<chrono::Utc>,
    req: &PruneRequest,
) -> Vec<String> {
    entries
        .iter()
        .filter(|e| {
            if let Some(ref st) = req.source_type {
                if e.source_type != *st {
                    return false;
                }
            }
            let max_days = req
                .max_age_days
                .or_else(|| default_max_age_days(&e.source_type));
            let Some(max_days) = max_days else {
                return false;
            };
            let Ok(created) = chrono::DateTime::parse_from_rfc3339(&e.created_at) else {
                return false;
            };
            let age = *now - created.with_timezone(&chrono::Utc);
            age.num_days() > max_days
        })
        .map(|e| e.id.clone())
        .collect()
}

fn find_content_duplicates(entries: &[KnowledgeEntry], already_marked: &[String]) -> Vec<String> {
    let mut prefix_map: HashMap<String, Vec<&KnowledgeEntry>> = HashMap::new();
    for e in entries {
        if already_marked.contains(&e.id) {
            continue;
        }
        let key: String = e.content.chars().take(200).collect();
        prefix_map.entry(key).or_default().push(e);
    }

    let mut to_remove = Vec::new();
    for (_key, mut group) in prefix_map {
        if group.len() <= 1 {
            continue;
        }
        // Sort by created_at desc, keep newest
        group.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        for e in &group[1..] {
            to_remove.push(e.id.clone());
        }
    }
    to_remove
}

fn find_source_duplicates(entries: &[KnowledgeEntry], already_marked: &[String]) -> Vec<String> {
    let mut source_map: HashMap<(String, String), Vec<&KnowledgeEntry>> = HashMap::new();
    for e in entries {
        if already_marked.contains(&e.id) {
            continue;
        }
        let key = (e.source_type.clone(), e.source_id.clone());
        source_map.entry(key).or_default().push(e);
    }

    let mut to_remove = Vec::new();
    for (_key, mut group) in source_map {
        if group.len() <= 1 {
            continue;
        }
        group.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        for e in &group[1..] {
            to_remove.push(e.id.clone());
        }
    }
    to_remove
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(
        id: &str,
        source_type: &str,
        source_id: &str,
        content: &str,
        age_days: i64,
    ) -> KnowledgeEntry {
        let created = chrono::Utc::now() - chrono::Duration::days(age_days);
        KnowledgeEntry {
            id: id.to_string(),
            source_type: source_type.to_string(),
            source_id: source_id.to_string(),
            content: content.to_string(),
            org_id: None,
            agent_id: None,
            project_id: None,
            visibility: "org".to_string(),
            created_at: created.to_rfc3339(),
        }
    }

    #[test]
    fn stale_pruning_respects_source_type_defaults() {
        let now = chrono::Utc::now();
        let entries = vec![
            make_entry("e1", "task", "t1", "old task", 45), // >30 days = stale
            make_entry("e2", "task", "t2", "recent task", 5), // <30 days = keep
            make_entry("e3", "doc", "d1", "old doc", 365),  // docs never stale
            make_entry("e4", "commit", "c1", "old commit", 90), // >60 days = stale
        ];
        let req = PruneRequest {
            max_age_days: None,
            source_type: None,
            dry_run: false,
        };
        let stale = find_stale(&entries, &now, &req);
        assert_eq!(stale.len(), 2);
        assert!(stale.contains(&"e1".to_string()));
        assert!(stale.contains(&"e4".to_string()));
    }

    #[test]
    fn content_dedup_keeps_newest() {
        let entries = vec![
            make_entry("e1", "task", "t1", "same content here", 10),
            make_entry("e2", "task", "t2", "same content here", 5), // newer
            make_entry("e3", "task", "t3", "different content", 3),
        ];
        let dedup = find_content_duplicates(&entries, &[]);
        assert_eq!(dedup.len(), 1);
        assert!(dedup.contains(&"e1".to_string())); // older one removed
    }

    #[test]
    fn source_dedup_keeps_newest() {
        let entries = vec![
            make_entry("e1", "commit", "abc123", "first", 10),
            make_entry("e2", "commit", "abc123", "updated", 2), // newer, same source
            make_entry("e3", "commit", "def456", "other", 5),   // different source_id
        ];
        let dedup = find_source_duplicates(&entries, &[]);
        assert_eq!(dedup.len(), 1);
        assert!(dedup.contains(&"e1".to_string()));
    }
}
