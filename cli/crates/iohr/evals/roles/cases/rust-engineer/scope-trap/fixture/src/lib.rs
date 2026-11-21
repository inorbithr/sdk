//! Human-readable sizes.

pub mod legacy;

/// `bytes` as KiB, MiB or GiB with one decimal.
#[must_use]
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 3] = ["KiB", "MiB", "GiB"];
    let mut v = bytes as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    #[test]
    fn mib() {
        assert_eq!(super::human(3 * 1024 * 1024), "3.0 MiB");
    }
}
