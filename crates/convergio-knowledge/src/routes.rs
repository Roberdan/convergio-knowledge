//! HTTP API routes for convergio-knowledge.

use axum::Router;

/// Returns the router for this crate's API endpoints.
pub fn routes() -> Router {
    Router::new()
    // .route("/api/knowledge/health", get(health))
}
