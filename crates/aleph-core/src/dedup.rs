//! Cosine-similarity and deduplication helpers.

/// Compute the cosine similarity between two equal-length vectors.
///
/// Returns a value in `[-1.0, 1.0]` (or `[0.0, 1.0]` for L2-normalized vectors).
/// Returns 0.0 if vectors have different dimensions (mismatch guard).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    dot / ((na.sqrt() * nb.sqrt()) + 1e-12)
}

/// Returns `true` if `new_vec` is too similar (≥ `threshold`) to any vector in `recent`.
pub fn should_dedup(new_vec: &[f32], recent: &[Vec<f32>], threshold: f32) -> bool {
    for prev in recent {
        if cosine_similarity(new_vec, prev) >= threshold {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn dedup_logic() {
        let new = vec![1.0, 0.0, 0.0];
        let recent = vec![vec![0.99, 0.01, 0.0]];
        assert!(should_dedup(&new, &recent, 0.95));
    }
}
