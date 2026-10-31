//! The scheduler's pick-next rule (priority then FIFO) and queue ETA.

use crate::Priority;

/// Pick the next job to start: highest priority, then lowest queue rank.
/// `candidates` is `(id, priority, rank)`.
pub fn pick_next(candidates: &[(String, Priority, i64)]) -> Option<String> {
    candidates
        .iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)))
        .map(|(id, _, _)| id.clone())
}

/// Estimate total queue time in seconds. `work` is `(audio_sec, rtf_est)` per
/// job. CPU parallelism is sublinear (jobs share cores), so effective
/// parallelism is `parallelism^0.8`.
pub fn queue_eta_secs(work: &[(f32, f32)], parallelism: u32) -> f64 {
    let total: f64 = work
        .iter()
        .map(|(audio, rtf)| (*audio as f64) * (*rtf as f64))
        .sum();
    let eff = (parallelism.max(1) as f64).powf(0.8).max(1.0);
    total / eff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_highest_priority_then_earliest() {
        let c = vec![
            ("a".into(), Priority::Normal, 100),
            ("b".into(), Priority::High, 200),
            ("c".into(), Priority::High, 150), // same priority, earlier
            ("d".into(), Priority::Low, 50),
        ];
        assert_eq!(pick_next(&c).as_deref(), Some("c"));
    }

    #[test]
    fn empty_queue_has_no_next() {
        assert_eq!(pick_next(&[]), None);
    }

    #[test]
    fn eta_shrinks_with_parallelism_but_sublinearly() {
        let work = vec![(600.0, 0.1); 4]; // 4 jobs, 60s work each = 240s total
        let one = queue_eta_secs(&work, 1);
        let four = queue_eta_secs(&work, 4);
        assert!((one - 240.0).abs() < 0.001);
        // 4x parallelism gives less than 4x speedup.
        assert!(four > one / 4.0 && four < one);
    }
}
