//! A per-account request quota.

/// What is left of an account's quota.
#[derive(Debug)]
pub struct Quota {
    left: u32,
}

impl Quota {
    /// A quota of `limit` requests.
    #[must_use]
    pub fn new(limit: u32) -> Self {
        Self { left: limit }
    }

    /// Requests left.
    #[must_use]
    pub fn left(&self) -> u32 {
        self.left
    }

    /// Charges `cost` requests, taken from the caller's batch size. Returns false, and
    /// charges nothing, when fewer than `cost` are left.
    pub fn charge(&mut self, cost: u32) -> bool {
        match self.left.checked_sub(cost) {
            Some(left) => {
                self.left = left;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charges() {
        let mut q = Quota::new(10);
        assert!(q.charge(4));
        assert_eq!(q.left(), 6);
    }

    #[test]
    fn a_batch_larger_than_what_is_left_is_refused() {
        let mut q = Quota::new(3);
        assert!(!q.charge(u32::MAX));
        assert_eq!(q.left(), 3);
    }
}
