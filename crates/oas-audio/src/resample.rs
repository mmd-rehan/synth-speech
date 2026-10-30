//! A small linear resampler. When downsampling it box-averages over the input
//! window to reduce aliasing; when upsampling it interpolates linearly.

/// Resample `input` from `src` Hz to `dst` Hz (single channel).
pub fn resample_linear(input: &[f32], src: u32, dst: u32) -> Vec<f32> {
    if src == dst || input.is_empty() {
        return input.to_vec();
    }
    let ratio = dst as f64 / src as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    let step = src as f64 / dst as f64; // input samples per output sample

    if dst < src {
        // Downsample: average over the window each output sample covers.
        let half = step / 2.0;
        for i in 0..out_len {
            let center = i as f64 * step;
            let lo = (center - half).max(0.0).floor() as usize;
            let hi = (((center + half).ceil() as usize) + 1).min(input.len());
            let lo = lo.min(input.len().saturating_sub(1));
            let hi = hi.max(lo + 1).min(input.len());
            let slice = &input[lo..hi];
            let avg = slice.iter().copied().sum::<f32>() / slice.len().max(1) as f32;
            out.push(avg);
        }
    } else {
        // Upsample: linear interpolation.
        for i in 0..out_len {
            let pos = i as f64 * step;
            let i0 = pos.floor() as usize;
            let frac = (pos - i0 as f64) as f32;
            let a = input.get(i0).copied().unwrap_or(0.0);
            let b = input.get(i0 + 1).copied().unwrap_or(a);
            out.push(a + (b - a) * frac);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_when_rates_match() {
        let x = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_linear(&x, 16000, 16000), x);
    }

    #[test]
    fn downsample_halves_length() {
        let x: Vec<f32> = (0..32000).map(|i| (i as f32).sin()).collect();
        let y = resample_linear(&x, 32000, 16000);
        // ~half the samples (within rounding).
        assert!((y.len() as i64 - 16000).abs() <= 2, "len = {}", y.len());
    }

    #[test]
    fn upsample_doubles_length() {
        let x: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        let y = resample_linear(&x, 8000, 16000);
        assert!((y.len() as i64 - 2000).abs() <= 2, "len = {}", y.len());
        // Monotonic ramp stays roughly monotonic.
        assert!(y[0] <= y[y.len() - 1]);
    }
}
