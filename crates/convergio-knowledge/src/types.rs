//! Types for the knowledge vector store.

use serde::{Deserialize, Serialize};

/// Allowed visibility values.
const VALID_VISIBILITIES: &[&str] = &["org", "public"];

/// Allowed source types.
const VALID_SOURCE_TYPES: &[&str] = &[
    "task",
    "commit",
    "doc",
    "agent_memory",
    "kb",
    "learning",
    "decision",
];

/// Maximum content length (64 KiB).
const MAX_CONTENT_LEN: usize = 65_536;

/// Maximum search limit per request.
const MAX_SEARCH_LIMIT: usize = 100;

/// Maximum query length for search requests.
const MAX_QUERY_LEN: usize = 2_000;

/// Validate that a visibility value is allowed.
pub fn validate_visibility(v: &str) -> Result<(), String> {
    if VALID_VISIBILITIES.contains(&v) {
        Ok(())
    } else {
        Err(format!(
            "invalid visibility '{v}', must be one of: {}",
            VALID_VISIBILITIES.join(", ")
        ))
    }
}

/// Validate that a source_type value is allowed.
pub fn validate_source_type(st: &str) -> Result<(), String> {
    if VALID_SOURCE_TYPES.contains(&st) {
        Ok(())
    } else {
        Err(format!(
            "invalid source_type '{st}', must be one of: {}",
            VALID_SOURCE_TYPES.join(", ")
        ))
    }
}

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

impl WriteRequest {
    /// Validate all fields, returning the first error found.
    pub fn validate(&self) -> Result<(), String> {
        if self.content.is_empty() {
            return Err("content must not be empty".into());
        }
        if self.content.len() > MAX_CONTENT_LEN {
            return Err(format!(
                "content too large ({} bytes, max {MAX_CONTENT_LEN})",
                self.content.len()
            ));
        }
        validate_source_type(&self.source_type)?;
        validate_visibility(&self.visibility)?;
        if self.source_id.is_empty() || self.source_id.len() > 256 {
            return Err("source_id must be 1-256 characters".into());
        }
        Ok(())
    }
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

impl SearchRequest {
    /// Validate and clamp fields.
    pub fn sanitize(&mut self) -> Result<(), String> {
        if self.query.is_empty() {
            return Err("query must not be empty".into());
        }
        if self.query.len() > MAX_QUERY_LEN {
            return Err(format!(
                "query too long ({} chars, max {MAX_QUERY_LEN})",
                self.query.len()
            ));
        }
        self.limit = self.limit.clamp(1, MAX_SEARCH_LIMIT);
        self.min_score = self.min_score.clamp(0.0, 1.0);
        if let Some(ref st) = self.source_type {
            validate_source_type(st)?;
        }
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_validation() {
        assert!(validate_visibility("org").is_ok());
        assert!(validate_visibility("public").is_ok());
        assert!(validate_visibility("admin").is_err());
        assert!(validate_visibility("").is_err());
    }

    #[test]
    fn source_type_validation() {
        assert!(validate_source_type("task").is_ok());
        assert!(validate_source_type("commit").is_ok());
        assert!(validate_source_type("doc").is_ok());
        assert!(validate_source_type("kb").is_ok());
        assert!(validate_source_type("agent_memory").is_ok());
        assert!(validate_source_type("learning").is_ok());
        assert!(validate_source_type("decision").is_ok());
        assert!(validate_source_type("'; DROP TABLE --").is_err());
        assert!(validate_source_type("").is_err());
    }

    #[test]
    fn write_request_rejects_empty_content() {
        let req = WriteRequest {
            content: String::new(),
            source_type: "doc".into(),
            source_id: "d1".into(),
            org_id: None,
            agent_id: None,
            project_id: None,
            visibility: "org".into(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn write_request_rejects_oversized_content() {
        let req = WriteRequest {
            content: "x".repeat(MAX_CONTENT_LEN + 1),
            source_type: "doc".into(),
            source_id: "d1".into(),
            org_id: None,
            agent_id: None,
            project_id: None,
            visibility: "org".into(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn write_request_rejects_invalid_visibility() {
        let req = WriteRequest {
            content: "test".into(),
            source_type: "doc".into(),
            source_id: "d1".into(),
            org_id: None,
            agent_id: None,
            project_id: None,
            visibility: "admin".into(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn search_request_clamps_limit() {
        let mut req = SearchRequest {
            query: "test".into(),
            limit: 99999,
            org_id: None,
            source_type: None,
            project_id: None,
            min_score: -5.0,
            federated: false,
        };
        assert!(req.sanitize().is_ok());
        assert_eq!(req.limit, MAX_SEARCH_LIMIT);
        assert_eq!(req.min_score, 0.0);
    }

    #[test]
    fn search_request_rejects_empty_query() {
        let mut req = SearchRequest {
            query: String::new(),
            limit: 5,
            org_id: None,
            source_type: None,
            project_id: None,
            min_score: 0.3,
            federated: false,
        };
        assert!(req.sanitize().is_err());
    }
}
