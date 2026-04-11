//! LanceDB vector store — embedding storage with HNSW index.
use std::path::Path;
use std::sync::Arc;

use arrow_array::{
    types::Float32Type, Array, FixedSizeListArray, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::StreamExt;
use lancedb::connection::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::types::{KnowledgeEntry, SearchResult, StoreStats};

pub const EMBEDDING_DIM: usize = 384;

pub struct LanceVectorStore {
    pub(crate) conn: Mutex<Connection>,
    pub(crate) table_name: String,
}

impl LanceVectorStore {
    pub async fn open(path: &Path) -> Result<Self, String> {
        let conn = lancedb::connect(path.to_str().unwrap_or("knowledge.lance"))
            .execute()
            .await
            .map_err(|e| format!("lance connect: {e}"))?;
        let store = Self {
            conn: Mutex::new(conn),
            table_name: "knowledge".into(),
        };
        store.ensure_table().await?;
        Ok(store)
    }

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("content", DataType::Utf8, false),
            Field::new("source_type", DataType::Utf8, false),
            Field::new("source_id", DataType::Utf8, false),
            Field::new("org_id", DataType::Utf8, true),
            Field::new("agent_id", DataType::Utf8, true),
            Field::new("project_id", DataType::Utf8, true),
            Field::new("visibility", DataType::Utf8, false),
            Field::new("created_at", DataType::Utf8, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    EMBEDDING_DIM as i32,
                ),
                false,
            ),
        ]))
    }

    async fn ensure_table(&self) -> Result<(), String> {
        let conn = self.conn.lock().await;
        let names = conn
            .table_names()
            .execute()
            .await
            .map_err(|e| format!("table_names: {e}"))?;
        if !names.contains(&self.table_name) {
            Self::create_empty_table(&conn, &self.table_name).await?;
        } else {
            // Schema evolution: drop and recreate if missing visibility column.
            let table = conn
                .open_table(&self.table_name)
                .execute()
                .await
                .map_err(|e| format!("open: {e}"))?;
            let has_visibility = table
                .schema()
                .await
                .map(|s| s.field_with_name("visibility").is_ok())
                .unwrap_or(false);
            if !has_visibility {
                info!("knowledge table missing visibility column, recreating");
                conn.drop_table(&self.table_name, &[])
                    .await
                    .map_err(|e| format!("drop table: {e}"))?;
                Self::create_empty_table(&conn, &self.table_name).await?;
            }
        }
        Ok(())
    }
    async fn create_empty_table(conn: &Connection, name: &str) -> Result<(), String> {
        let schema = Self::schema();
        let batch = RecordBatch::new_empty(schema.clone());
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        let reader: Box<dyn RecordBatchReader + Send> = Box::new(batches);
        conn.create_table(name, reader)
            .execute()
            .await
            .map_err(|e| format!("create table: {e}"))?;
        info!("created knowledge lance table");
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert(
        &self,
        content: &str,
        source_type: &str,
        source_id: &str,
        org_id: Option<&str>,
        agent_id: Option<&str>,
        project_id: Option<&str>,
        visibility: &str,
        embedding: &[f32],
    ) -> Result<String, String> {
        if embedding.len() != EMBEDDING_DIM {
            return Err(format!(
                "embedding dim {}, expected {EMBEDDING_DIM}",
                embedding.len()
            ));
        }
        let id = format!("ke-{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();

        let schema = Self::schema();
        let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            vec![Some(embedding.iter().map(|v| Some(*v)).collect::<Vec<_>>())],
            EMBEDDING_DIM as i32,
        );
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![id.as_str()])),
                Arc::new(StringArray::from(vec![content])),
                Arc::new(StringArray::from(vec![source_type])),
                Arc::new(StringArray::from(vec![source_id])),
                Arc::new(StringArray::from(vec![org_id.unwrap_or("")])),
                Arc::new(StringArray::from(vec![agent_id.unwrap_or("")])),
                Arc::new(StringArray::from(vec![project_id.unwrap_or("")])),
                Arc::new(StringArray::from(vec![visibility])),
                Arc::new(StringArray::from(vec![now.as_str()])),
                Arc::new(vectors),
            ],
        )
        .map_err(|e| format!("build batch: {e}"))?;

        let conn = self.conn.lock().await;
        let table = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open table: {e}"))?;
        let batches = RecordBatchIterator::new(vec![Ok(batch)], schema);
        let reader: Box<dyn RecordBatchReader + Send> = Box::new(batches);
        table
            .add(reader)
            .execute()
            .await
            .map_err(|e| format!("insert: {e}"))?;

        info!(id = %id, source_type, "knowledge entry stored via LanceDB");
        Ok(id)
    }

    pub async fn search(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<SearchResult>, String> {
        let conn = self.conn.lock().await;
        let table = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open: {e}"))?;

        let count = table.count_rows(None).await.unwrap_or(0);
        if count == 0 {
            return Ok(vec![]);
        }

        let stream = table
            .query()
            .nearest_to(query_embedding)
            .map_err(|e| format!("nearest_to: {e}"))?
            .limit(limit)
            .execute()
            .await
            .map_err(|e| format!("execute: {e}"))?;

        let mut out = Vec::new();
        let mut stream = stream;
        while let Some(batch_result) = stream.next().await {
            let batch = match batch_result {
                Ok(b) => b,
                Err(e) => {
                    warn!(error = %e, "lance stream error");
                    continue;
                }
            };
            let col = |name: &str| -> Option<&StringArray> {
                batch.column_by_name(name)?.as_any().downcast_ref()
            };
            let (Some(ids), Some(contents), Some(src_types), Some(src_ids)) = (
                col("id"),
                col("content"),
                col("source_type"),
                col("source_id"),
            ) else {
                warn!("missing columns in lance result");
                continue;
            };
            let org_ids = col("org_id");
            let agent_ids = col("agent_id");
            let project_ids = col("project_id");
            let visibilities = col("visibility");
            let created = col("created_at");
            let distances = batch
                .column_by_name("_distance")
                .and_then(|c| c.as_any().downcast_ref::<arrow_array::Float32Array>());

            for i in 0..batch.num_rows() {
                let dist = distances.map(|d| d.value(i)).unwrap_or(0.0);
                let score = 1.0 / (1.0 + dist as f64);
                out.push(SearchResult {
                    entry: KnowledgeEntry {
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
                    },
                    score,
                });
            }
        }
        Ok(out)
    }
    pub async fn stats(&self) -> Result<StoreStats, String> {
        let conn = self.conn.lock().await;
        let table = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open: {e}"))?;
        let count = table
            .count_rows(None)
            .await
            .map_err(|e| format!("count: {e}"))?;
        Ok(StoreStats {
            total_entries: count as i64,
            total_by_source: vec![],
            embedding_dimensions: EMBEDDING_DIM,
        })
    }

    pub async fn delete(&self, id: &str) -> Result<bool, String> {
        let conn = self.conn.lock().await;
        let table = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open: {e}"))?;
        let id_esc = id.replace('\'', "''");
        table
            .delete(&format!("id = '{id_esc}'"))
            .await
            .map_err(|e| format!("delete: {e}"))?;
        Ok(true)
    }

    pub async fn count_by_source(
        &self,
        source_type: &str,
        source_id: &str,
    ) -> Result<usize, String> {
        let conn = self.conn.lock().await;
        let table = conn
            .open_table(&self.table_name)
            .execute()
            .await
            .map_err(|e| format!("open: {e}"))?;
        let (st, si) = (
            source_type.replace('\'', "''"),
            source_id.replace('\'', "''"),
        );
        let count = table
            .count_rows(Some(format!("source_type = '{st}' AND source_id = '{si}'")))
            .await
            .map_err(|e| format!("count_by_source: {e}"))?;
        Ok(count)
    }
}
