// Modified by Whispertype: reusable, score-compatible rolling DTW workspace.
#[cfg(test)]
use super::dtw::Dtw;
use std::{cell::RefCell, cmp};

#[derive(Default)]
struct Rows {
    previous: Vec<f32>,
    current: Vec<f32>,
    a_squared: Vec<f32>,
    b_squared: Vec<f32>,
}

pub struct MfccComparator {
    score_ref: f32,
    band_size: u16,
    rows: RefCell<Rows>,
}
impl MfccComparator {
    pub fn new(score_ref: f32, band_size: u16) -> Self {
        MfccComparator {
            score_ref,
            band_size,
            rows: RefCell::new(Rows::default()),
        }
    }
    pub fn calculate_distance(ax: &[f32], bx: &[f32]) -> f32 {
        1. - cosine_similarity(ax, bx)
    }
    pub fn compare<T: AsRef<[f32]>>(&self, a: &[T], b: &[T]) -> f32 {
        let cost = self.rolling_cost(a, b);
        let normalized_cost = cost / (a.len() + b.len()) as f32;
        self.compute_probability(normalized_cost)
    }
    fn rolling_cost<T: AsRef<[f32]>>(&self, a: &[T], b: &[T]) -> f32 {
        assert!(!a.is_empty() && !b.is_empty());
        let window = (self.band_size as usize).max(a.len().abs_diff(b.len()));
        let mut rows = self.rows.borrow_mut();
        let Rows {
            previous,
            current,
            a_squared,
            b_squared,
        } = &mut *rows;
        let dimensions = a[0].as_ref().len();
        let uniform = a.iter().all(|row| row.as_ref().len() == dimensions)
            && b.iter().all(|row| row.as_ref().len() == dimensions);
        if uniform {
            a_squared.resize(a.len(), 0.);
            b_squared.resize(b.len(), 0.);
            for (target, vector) in a_squared.iter_mut().zip(a) {
                *target = squared_magnitude(vector.as_ref());
            }
            for (target, vector) in b_squared.iter_mut().zip(b) {
                *target = squared_magnitude(vector.as_ref());
            }
        }
        previous.resize(b.len() + 1, f32::INFINITY);
        current.resize(b.len() + 1, f32::INFINITY);
        previous.fill(f32::INFINITY);
        current.fill(f32::INFINITY);
        previous[0] = 0.;
        // Upstream 3.0.2 returns D[m-1][n], not D[m][n], after copying
        // its matrix. Keep that exact terminal cell and score scale.
        for row in 1..a.len() {
            let start = row.saturating_sub(window).max(1);
            let end = (b.len() + 1).min(row + window);
            current[start - 1] = f32::INFINITY;
            for column in start..end {
                let cost = if uniform {
                    let mut dot = 0.;
                    for (a, b) in a[row - 1].as_ref().iter().zip(b[column - 1].as_ref()) {
                        dot += a * b;
                    }
                    let magnitude = (a_squared[row - 1] * b_squared[column - 1]).sqrt();
                    1. - if magnitude == 0. { 0. } else { dot / magnitude }
                } else {
                    Self::calculate_distance(a[row - 1].as_ref(), b[column - 1].as_ref())
                };
                let best = [previous[column], current[column - 1], previous[column - 1]]
                    .iter()
                    .fold(f32::INFINITY, |a, &b| a.min(b));
                current[column] = cost + best;
            }
            if end <= b.len() {
                current[end] = f32::INFINITY;
            }
            std::mem::swap(previous, current);
        }
        previous[b.len()]
    }
    fn compute_probability(&self, cost: f32) -> f32 {
        1. / (1. + ((cost - self.score_ref) / self.score_ref).exp())
    }
}

fn squared_magnitude(vector: &[f32]) -> f32 {
    let mut sum = 0.;
    for value in vector {
        sum += value * value;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rolling_dtw_matches_upstream_terminal_cell_bit_for_bit() {
        for m in [1usize, 2, 5, 17, 40, 80] {
            for n in [1usize, 2, 5, 17, 40, 80] {
                let a: Vec<Vec<f32>> = (0..m)
                    .map(|i| {
                        (0..16)
                            .map(|j| ((i * 17 + j) as f32 * 0.13).sin())
                            .collect()
                    })
                    .collect();
                let b: Vec<Vec<f32>> = (0..n)
                    .map(|i| {
                        (0..16)
                            .map(|j| ((i * 11 + j) as f32 * 0.09).cos())
                            .collect()
                    })
                    .collect();
                let a: Vec<_> = a.iter().map(Vec::as_slice).collect();
                let b: Vec<_> = b.iter().map(Vec::as_slice).collect();
                for band in [0, 1, 5, 10] {
                    let optimized = MfccComparator::new(0.22, band);
                    let mut reference = Dtw::new(MfccComparator::calculate_distance);
                    let expected = reference.compute_optimal_path_with_window(&a, &b, band);
                    for _ in 0..3 {
                        assert_eq!(
                            optimized.rolling_cost(&a, &b).to_bits(),
                            expected.to_bits(),
                            "m={m} n={n} band={band}"
                        );
                    }
                }
            }
        }
    }
}
pub fn cosine_similarity(vector_a: &[f32], vector_b: &[f32]) -> f32 {
    let dimensionality = cmp::min(vector_a.len(), vector_b.len());
    let mut dot_ab = 0.;
    let mut dot_a = 0.;
    let mut dot_b = 0.;
    let mut dimension = 0;
    while dimension < dimensionality {
        let component_a = vector_a[dimension];
        let component_b = vector_b[dimension];
        dot_ab += component_a * component_b;
        dot_a += component_a * component_a;
        dot_b += component_b * component_b;
        dimension += 1;
    }
    let magnitude = f32::sqrt(dot_a * dot_b);
    if magnitude == 0. {
        0.
    } else {
        dot_ab / magnitude
    }
}
