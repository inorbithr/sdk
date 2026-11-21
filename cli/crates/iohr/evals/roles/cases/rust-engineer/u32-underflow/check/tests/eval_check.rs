//! A batch larger than the quota is refused, never wrapped or panicked on.

use quota::Quota;

#[test]
fn eval_large_cost_is_refused() {
    for (limit, cost) in [(3, 4), (1, u32::MAX), (10, 11)] {
        let r = std::panic::catch_unwind(move || {
            let mut q = Quota::new(limit);
            (q.charge(cost), q.left())
        });
        assert_eq!(r.ok(), Some((false, limit)), "charge({cost}) on {limit}");
    }
}
