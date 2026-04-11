//! Types for the knowledge vector store.

use serde::{Deserialize, Serialize};

/// A knowledge entry stored with its embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub id: String,
    /// Source type: "task", "commit", "doc", "agent_memory", "kb"
    pub source_type: String,
    /// Source identifier (task_id, commit hash, doc path, etc.)
    pub source_id: String,
    /// The text content that was embedded.
    pub content: String,
    /// Optional org scope.
    pub org_id: Option<String>,
    /// Optional agent that produced this.
    pub agent_id: Option<String>,
    /// Optional project/repo scope (e.g. "convergio", "istitutodeimpresa").
    pub project_id: Option<String>,
    /// Visibility: "org" (default, only within org) or "public" (cross-org federation).
    #[serde(default = "default_visibility")]
    pub visibility: String,
    /// ISO 8601 timestamp.
    pub created_at: String,
}

/// Result of a vector similarity search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub entry: KnowledgeEntry,
    /// Cosine similarity score (0.0 to 1.0).
    pub score: f64,
}

/// Request to write a knowledge entry.
#[derive(Debug, Deserialize)]
pub struct WriteRequest {
    pub content: String,
    pub source_type: String,
    pub source_id: String,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Visibility: "org" (default) or "public".
    #[serde(default = "default_visibility")]
    pub visibility: String,
}

fn default_visibility() -> String {
    "org".into()
}

/// Request to search knowledge.
#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Minimum score threshold (0.0-1.0). Results below this are discarded.
    #[serde(default = "default_min_score")]
    pub min_score: f64,
    /// If true, include public entries from all orgs (cross-org federation).
    #[serde(default)]
    pub federated: bool,
}

fn default_limit() -> usize {
    5
}

fn default_min_score() -> f64 {
    0.3
}

/// Stats about the knowledge store.
#[derive(Debug, Serialize)]
pub struct StoreStats {
    pub total_entries: i64,
    pub total_by_source: Vec<(String, i64)>,
    pub embedding_dimensions: usize,
}
