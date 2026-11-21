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

    /// Charges `cost` requests, taken from the caller's batch size. Returns false when the
    /// quota is spent.
    pub fn charge(&mut self, cost: u32) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= cost;
        true
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
}
