//! HTTP routes for the knowledge vector store.
//!
//! - POST /api/knowledge/search  — semantic search
//! - POST /api/knowledge/write   — store a knowledge entry
//! - GET  /api/knowledge/stats   — store statistics
//! - DELETE /api/knowledge/:id   — delete an entry

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use convergio_db::pool::ConnPool;
use serde_json::{json, Value};

use crate::store::LanceVectorStore;
use crate::types::{SearchRequest, WriteRequest};

pub struct KnowledgeState {
    pub pool: ConnPool,
    pub store: Option<Arc<LanceVectorStore>>,
}

pub fn knowledge_routes(state: Arc<KnowledgeState>) -> Router {
    Router::new()
        .route("/api/knowledge/search", post(handle_search))
        .route("/api/knowledge/write", post(handle_write))
        .route("/api/knowledge/stats", get(handle_stats))
        .route("/api/knowledge/seed", post(handle_seed))
        .route("/api/knowledge/prune", post(handle_prune))
        .route("/api/knowledge/:id", delete(handle_delete))
        .with_state(state)
}

async fn handle_search(
    State(s): State<Arc<KnowledgeState>>,
    Json(mut req): Json<SearchRequest>,
) -> Json<Value> {
    if let Err(e) = req.sanitize() {
        return Json(json!({"error": e}));
    }
    let embedding = crate::embedder::embed(&req.query).await;
    let fetch_limit =
        if req.org_id.is_some() || req.source_type.is_some() || req.project_id.is_some() {
            req.limit * 3
        } else {
            req.limit
        };

    let Some(ref store) = s.store else {
        return Json(json!({"results": [], "error": "knowledge store not available"}));
    };
    match store.search(&embedding, fetch_limit).await {
        Ok(mut results) => {
            // Filter by org — when federated, also include public entries
            if let Some(ref org) = req.org_id {
                if req.federated {
                    results.retain(|r| {
                        r.entry.org_id.as_deref() == Some(org.as_str())
                            || r.entry.visibility == "public"
                    });
                } else {
                    results.retain(|r| r.entry.org_id.as_deref() == Some(org.as_str()));
                }
            }
            if let Some(ref st) = req.source_type {
                results.retain(|r| r.entry.source_type == *st);
            }
            if let Some(ref proj) = req.project_id {
                results.retain(|r| r.entry.project_id.as_deref() == Some(proj.as_str()));
            }
            // Filter by minimum score threshold
            results.retain(|r| r.score >= req.min_score);
            // Deduplicate by content prefix (first 200 chars)
            let mut seen = std::collections::HashSet::new();
            results.retain(|r| {
                let key: String = r.entry.content.chars().take(200).collect();
                seen.insert(key)
            });
            results.truncate(req.limit);
            // Truncate content to save tokens — full content available via /api/knowledge/:id
            for r in &mut results {
                if r.entry.content.len() > 200 {
                    r.entry.content = r.entry.content.chars().take(200).collect::<String>() + "…";
                }
            }
            let count = results.len();
            Json(json!({
                "results": results,
                "query": req.query,
                "count": count,
                "min_score": req.min_score,
            }))
        }
        Err(e) => Json(json!({"error": e})),
    }
}

async fn handle_write(
    State(s): State<Arc<KnowledgeState>>,
    Json(req): Json<WriteRequest>,
) -> Json<Value> {
    if let Err(e) = req.validate() {
        return Json(json!({"error": e}));
    }
    let Some(ref store) = s.store else {
        return Json(json!({"error": "knowledge store not available"}));
    };
    let embedding = crate::embedder::embed(&req.content).await;

    match store
        .insert(
            &req.content,
            &req.source_type,
            &req.source_id,
            req.org_id.as_deref(),
            req.agent_id.as_deref(),
            req.project_id.as_deref(),
            &req.visibility,
            &embedding,
        )
        .await
    {
        Ok(id) => Json(json!({"id": id, "status": "stored"})),
        Err(e) => Json(json!({"error": e})),
    }
}

async fn handle_stats(State(s): State<Arc<KnowledgeState>>) -> Json<Value> {
    let Some(ref store) = s.store else {
        return Json(json!({"error": "knowledge store not available"}));
    };
    match store.stats().await {
        Ok(stats) => Json(json!(stats)),
        Err(e) => Json(json!({"error": e})),
    }
}

async fn handle_delete(
    State(s): State<Arc<KnowledgeState>>,
    Path(id): Path<String>,
) -> Json<Value> {
    if !id.starts_with("ke-") || id.len() > 64 {
        return Json(json!({"error": "invalid knowledge entry id"}));
    }
    let Some(ref store) = s.store else {
        return Json(json!({"error": "knowledge store not available"}));
    };
    match store.delete(&id).await {
        Ok(true) => Json(json!({"deleted": true})),
        Ok(false) => Json(json!({"error": "not found"})),
        Err(e) => Json(json!({"error": e})),
    }
}

/// POST /api/knowledge/seed — ingest all known knowledge sources into vectors.
async fn handle_seed(State(s): State<Arc<KnowledgeState>>) -> Json<Value> {
    let Some(ref store) = s.store else {
        return Json(json!({"error": "knowledge store not available"}));
    };
    let report = crate::seed::seed_baseline(store, &s.pool).await;
    Json(json!({
        "total": report.total,
        "by_source": report.by_source,
        "skipped": report.skipped,
        "errors": report.errors
    }))
}

/// POST /api/knowledge/prune — auto-prune stale and duplicate entries (#688).
async fn handle_prune(
    State(s): State<Arc<KnowledgeState>>,
    Json(req): Json<crate::pruning::PruneRequest>,
) -> Json<Value> {
    if let Err(e) = req.validate() {
        return Json(json!({"error": e}));
    }
    let Some(ref store) = s.store else {
        return Json(json!({"error": "knowledge store not available"}));
    };
    let report = crate::pruning::prune(store, &req).await;
    Json(json!(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_deserializes() {
        let json = r#"{"query": "agent task", "limit": 10}"#;
        let req: SearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query, "agent task");
        assert_eq!(req.limit, 10);
    }

    #[test]
    fn write_request_deserializes() {
        let json = r#"{"content": "test", "source_type": "doc", "source_id": "d1"}"#;
        let req: WriteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.source_type, "doc");
        assert!(req.org_id.is_none());
    }
}
