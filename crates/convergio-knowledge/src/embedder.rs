//! Pure Rust embedding via fastembed (ONNX Runtime, no Python).
//!
//! Downloads model on first use (~25MB for AllMiniLML6V2).
//! Produces 384-dim embeddings in-process with zero external dependencies.

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::OnceLock;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Shared model instance — loaded once, reused across calls.
static MODEL: OnceLock<Mutex<TextEmbedding>> = OnceLock::new();

fn get_model() -> &'static Mutex<TextEmbedding> {
    MODEL.get_or_init(|| {
        info!("loading embedding model AllMiniLML6V2...");
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(true),
        )
        .expect("failed to load embedding model");
        info!("embedding model loaded (384 dims)");
        Mutex::new(model)
    })
}

/// Embed a single text. Returns 384-dim f32 vector.
pub async fn embed(text: &str) -> Vec<f32> {
    let model = get_model();
    let mut guard = model.lock().await;
    match guard.embed(vec![text.to_string()], None) {
        Ok(mut results) if !results.is_empty() => results.remove(0),
        Ok(_) => {
            warn!("embedding returned empty results");
            vec![0.0; 384]
        }
        Err(e) => {
            warn!(error = %e, "embedding failed");
            vec![0.0; 384]
        }
    }
}

/// Embed multiple texts in batch. More efficient than individual calls.
pub async fn embed_batch(texts: &[String]) -> Vec<Vec<f32>> {
    if texts.is_empty() {
        return vec![];
    }
    let model = get_model();
    let mut guard = model.lock().await;
    match guard.embed(texts, None) {
        Ok(results) => results,
        Err(e) => {
            warn!(error = %e, "batch embedding failed");
            texts.iter().map(|_| vec![0.0; 384]).collect()
        }
    }
}

/// Embedding dimension (AllMiniLML6V2 = 384).
pub const fn embedding_dim() -> usize {
    384
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn embed_produces_384_dims() {
        let v = embed("test convergio knowledge store").await;
        assert_eq!(v.len(), 384);
        // Should not be all zeros (model loaded successfully)
        let nonzero = v.iter().any(|x| *x != 0.0);
        assert!(nonzero, "embedding should have non-zero values");
    }

    #[tokio::test]
    async fn similar_texts_have_high_similarity() {
        let a = embed("rate limiter fix for authenticated agents").await;
        let b = embed("fixing the rate limit for auth users").await;
        let c = embed("chocolate cake recipe with vanilla frosting").await;

        let sim_ab = cosine(&a, &b);
        let sim_ac = cosine(&a, &c);
        assert!(
            sim_ab > sim_ac,
            "similar texts should score higher: ab={sim_ab:.3} ac={sim_ac:.3}"
        );
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if na == 0.0 || nb == 0.0 {
            0.0
        } else {
            dot / (na * nb)
        }
    }
}
