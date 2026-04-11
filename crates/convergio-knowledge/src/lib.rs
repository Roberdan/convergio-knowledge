//! convergio-knowledge — Vector knowledge store for semantic agent memory.
//!
//! Provides embedding-based search over agent knowledge:
//! task summaries, commit messages, org knowledge, and documentation.
//! Uses LanceDB for vector storage with HNSW index.
//! Embeddings generated via MLX local inference.

pub mod embedder;
pub mod ext;
pub mod hooks;
pub mod mcp_defs;
pub mod pruning;
pub mod routes;
pub mod seed;
pub mod seed_parsers;
pub mod store;
pub mod store_pruning;
pub mod types;

pub use ext::KnowledgeExtension;
pub use store::LanceVectorStore;
pub use types::{KnowledgeEntry, SearchResult};
