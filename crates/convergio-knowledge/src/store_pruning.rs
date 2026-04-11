//! Pruning helpers for the vector store — list_all_metadata + delete_batch.
//!
//! Extracted from store.rs to stay under the 300-line limit.

use std::collections::HashSet;

use arrow_array::StringArray;
use futures::StreamExt;
use lancedb::query::ExecutableQuery;

use crate::store::LanceVectorStore;
use crate::types::KnowledgeEntry;

impl LanceVectorStore {
    /// Collect all source_ids in the store (used by seed dedup).
    pub async fn all_source_ids(&self) -> Result<HashSet<String>, String> {
        let conn = self.conn.lock().await;
        let tbl = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open: {e}"))?;
        if tbl.count_rows(None).await.unwrap_or(0) == 0 {
            return Ok(HashSet::new());
        }
        let mut stream = tbl
            .query()
            .execute()
            .await
            .map_err(|e| format!("query source_ids: {e}"))?;
        let mut ids = HashSet::new();
        while let Some(Ok(batch)) = stream.next().await {
            if let Some(col) = batch
                .column_by_name("source_id")
                .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            {
                for i in 0..batch.num_rows() {
                    ids.insert(col.value(i).to_string());
                }
            }
        }
        Ok(ids)
    }

    /// List all entries (lightweight metadata only — no vectors).
    /// Used by the pruning system to scan for stale/duplicate entries.
    pub async fn list_all_metadata(&self) -> Result<Vec<KnowledgeEntry>, String> {
        let conn = self.conn.lock().await;
        let tbl = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open: {e}"))?;
        if tbl.count_rows(None).await.unwrap_or(0) == 0 {
            return Ok(vec![]);
        }
        let mut stream = tbl
            .query()
            .execute()
            .await
            .map_err(|e| format!("query all: {e}"))?;

        let mut out = Vec::new();
        while let Some(Ok(batch)) = stream.next().await {
            let col = |name: &str| -> Option<&StringArray> {
                batch.column_by_name(name)?.as_any().downcast_ref()
            };
            let (Some(ids), Some(contents), Some(src_types), Some(src_ids)) = (
                col("id"),
                col("content"),
                col("source_type"),
                col("source_id"),
            ) else {
                continue;
            };
            let org_ids = col("org_id");
            let agent_ids = col("agent_id");
            let project_ids = col("project_id");
            let visibilities = col("visibility");
            let created = col("created_at");

            for i in 0..batch.num_rows() {
                out.push(KnowledgeEntry {
                    id: ids.value(i).to_string(),
                    content: contents.value(i).to_string(),
                    source_type: src_types.value(i).to_string(),
                    source_id: src_ids.value(i).to_string(),
                    org_id: org_ids.map(|a| a.value(i).to_string()),
                    agent_id: agent_ids.map(|a| a.value(i).to_string()),
                    project_id: project_ids.map(|a| a.value(i).to_string()),
                    visibility: visibilities
                        .map(|a| a.value(i).to_string())
                        .unwrap_or_else(|| "org".to_string()),
                    created_at: created.map(|a| a.value(i).to_string()).unwrap_or_default(),
                });
            }
        }
        Ok(out)
    }

    /// Delete multiple entries by ID in a single filter expression.
    pub async fn delete_batch(&self, ids: &[String]) -> Result<usize, String> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().await;
        let table = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open: {e}"))?;
        let escaped: Vec<String> = ids
            .iter()
            .map(|id| format!("'{}'", id.replace('\'', "''")))
            .collect();
        let filter = format!("id IN ({})", escaped.join(", "));
        table
            .delete(&filter)
            .await
            .map_err(|e| format!("delete_batch: {e}"))?;
        Ok(ids.len())
    }
}
