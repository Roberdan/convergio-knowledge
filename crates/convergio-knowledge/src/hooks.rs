//! Hooks — auto-embed task summaries and commit messages.
//!
//! - `on_task_completed`: fires on TaskCompleted domain event,
//!   reads the task summary from DB and embeds it.
//! - `sync_recent_commits`: scheduled task that scans recent git
//!   commits and embeds their messages.

use std::sync::Arc;

use convergio_db::pool::ConnPool;
use tracing::{info, warn};

use crate::store::LanceVectorStore;

/// Called when a TaskCompleted event fires.
/// Reads the task summary and writes it to the knowledge store.
pub fn on_task_completed(pool: &ConnPool, store: Arc<LanceVectorStore>, task_id: i64) {
    let conn = match pool.get() {
        Ok(c) => c,
        Err(e) => {
            warn!(task_id, error = %e, "knowledge hook: db error");
            return;
        }
    };

    let row: Result<(String, Option<String>, Option<String>), _> = conn.query_row(
        "SELECT title, summary, notes FROM tasks WHERE id = ?1",
        [task_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    );

    let (title, summary, notes) = match row {
        Ok(r) => r,
        Err(e) => {
            warn!(task_id, error = %e, "task not found for knowledge embed");
            return;
        }
    };

    let content = format!(
        "Task completed: {title}\n{}{}",
        summary
            .as_deref()
            .map(|s| format!("Summary: {s}\n"))
            .unwrap_or_default(),
        notes
            .as_deref()
            .map(|n| format!("Notes: {n}\n"))
            .unwrap_or_default(),
    );

    let source_id = format!("task-{task_id}");
    tokio::spawn(async move {
        let embedding = crate::embedder::embed(&content).await;
        match store
            .insert(
                &content, "task", &source_id, None, None, None, "org", &embedding,
            )
            .await
        {
            Ok(id) => info!(id, task_id, "task knowledge embedded"),
            Err(e) => warn!(task_id, error = %e, "embed task failed"),
        }
    });
}

/// Sync recent git commits — called on schedule (every 15 min).
/// Reads the repo's recent commits and embeds their messages.
pub fn sync_recent_commits(store: Arc<LanceVectorStore>) {
    let repo_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let output = match std::process::Command::new("git")
        .args(["log", "--oneline", "-20", "--format=%H|%s"])
        .current_dir(&repo_root)
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            warn!(error = %stderr, "git log failed");
            return;
        }
        Err(e) => {
            warn!(error = %e, "git not available");
            return;
        }
    };

    let commits: Vec<(String, String)> = output
        .lines()
        .filter_map(|line| {
            let (hash, msg) = line.split_once('|')?;
            Some((hash.to_string(), msg.to_string()))
        })
        .collect();

    if commits.is_empty() {
        return;
    }

    tokio::spawn(async move {
        let mut embedded = 0u32;
        for (hash, msg) in &commits {
            let exists = store.count_by_source("commit", hash).await.unwrap_or(0);
            if exists > 0 {
                continue;
            }
            let embedding = crate::embedder::embed(msg).await;
            if store
                .insert(msg, "commit", hash, None, None, None, "org", &embedding)
                .await
                .is_ok()
            {
                embedded += 1;
            }
        }
        if embedded > 0 {
            info!(embedded, "commit knowledge synced");
        }
    });
}
