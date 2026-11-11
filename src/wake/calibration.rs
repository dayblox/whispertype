use super::*;

/// DTW/cosine score using the reference detector's band and score scale.
/// Rustpotter 3 normalizes this cost by both sequence lengths and maps it
/// through a sigmoid (reference 0.22). A perfect match is about 0.731, not 1.
fn similarity(a: &[Vec<f32>], b: &[Vec<f32>]) -> f32 {
    let window = 5.max(a.len().abs_diff(b.len()));
    let mut previous = vec![f32::INFINITY; b.len() + 1];
    let mut current = previous.clone();
    previous[0] = 0.;
    for (i, x) in a.iter().enumerate() {
        current.fill(f32::INFINITY);
        let row = i + 1;
        for column in row.saturating_sub(window).max(1)..(b.len() + 1).min(row + window) {
            let y = &b[column - 1];
            let dot = x.iter().zip(y).map(|(x, y)| x * y).sum::<f32>();
            let norm = (x.iter().map(|v| v * v).sum::<f32>()
                * y.iter().map(|v| v * v).sum::<f32>())
            .sqrt();
            let cosine = if norm > 0. { dot / norm } else { 0. };
            current[column] = 1. - cosine
                + previous[column]
                    .min(current[column - 1])
                    .min(previous[column - 1]);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let cost = previous[b.len()] / (a.len() + b.len()) as f32;
    1. / (1. + ((cost - 0.22) / 0.22).exp())
}

pub(super) fn threshold(model: &WakewordRef) -> Result<f32> {
    let samples: Vec<_> = model.samples_features.values().collect();
    ensure!((3..=8).contains(&samples.len()), "Automatic wake sensitivity requires three to eight personal recordings. Record your phrase again.");
    ensure!(
        samples.iter().all(|sample| !sample.is_empty()
            && sample.len() <= 320
            && sample
                .iter()
                .all(|row| row.len() == model.mfcc_size as usize
                    && row.iter().all(|v| v.is_finite()))),
        "Invalid personal phrase features. Record your phrase again."
    );
    // Leave each recording out and find its nearest OTHER recording. A strong
    // self match cannot conceal one sample that differs from all the others.
    let mut nearest = vec![0f32; samples.len()];
    for i in 0..samples.len() {
        for j in i + 1..samples.len() {
            let score = similarity(samples[i], samples[j]).min(similarity(samples[j], samples[i]));
            nearest[i] = nearest[i].max(score);
            nearest[j] = nearest[j].max(score);
        }
    }
    let weakest = nearest.into_iter().fold(f32::INFINITY, f32::min);
    ensure!(weakest >= 0.50, "These recordings are too inconsistent to enable reliably. Re-record the same phrase at a consistent distance.");
    // Leave a small variation margin without lowering the target indefinitely
    // for poor recordings. These are detector scores, not accuracy estimates.
    Ok((weakest - 0.04).clamp(0.50, 0.60))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silent_or_corrupt_references_cannot_weaken_the_automatic_target() {
        let mut model = WakewordRef {
            name: "Invalid sample".into(),
            mfcc_size: 16,
            rms_level: 0.,
            threshold: None,
            avg_threshold: None,
            avg_features: None,
            samples_features: (0..3)
                .map(|i| (format!("{i}.wav"), vec![vec![0.; 16]; 40]))
                .collect(),
        };
        assert!(threshold(&model).is_err());
        model.samples_features.get_mut("0.wav").unwrap()[0][0] = f32::NAN;
        assert!(threshold(&model).is_err());
    }
}
