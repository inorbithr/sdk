//! Small statistics for latency samples, in milliseconds.

/// The mean of `xs`, or `None` when it is empty.
#[must_use]
pub fn mean(xs: &[f64]) -> Option<f64> {
    (!xs.is_empty()).then(|| xs.iter().sum::<f64>() / xs.len() as f64)
}

/// The median of `xs`, or `None` when it is empty.
#[must_use]
pub fn median(xs: &[f64]) -> Option<f64> {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    match v.len() {
        0 => None,
        n if n % 2 == 0 => Some((v[mid - 1] + v[mid]) / 2.0),
        _ => Some(v[mid]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mean_of_three() {
        assert_eq!(mean(&[1.0, 2.0, 6.0]), Some(3.0));
    }

    #[test]
    fn median_of_an_even_count_is_the_mean_of_the_middle_two() {
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }
}
