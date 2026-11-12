//! Independence (ADR 0009): evidence references, lineage and failure modes come from
//! the `item` module (re-exported here); the grouping that decides what counts as
//! corroboration stays private.
//!
//! Corroboration counts only between independent sources. Two pieces of evidence are
//! dependent when they share an ancestor (four dashboards on one trace) or a failure
//! mode (the same host clock, the same kernel, the same extraction model). Independence
//! is computed, never asserted: evidence is grouped by connected components over shared
//! ancestors and failure modes, and proof rules count groups, not items.

use std::collections::BTreeMap;

mod item;

pub use item::*;

fn roots(item: &EvidenceItem) -> impl Iterator<Item = Key> + '_ {
    std::iter::once(Key::Ref(item.reference))
        .chain(item.ancestors.iter().copied().map(Key::Ref))
        .chain(item.failure_modes.iter().cloned().map(Key::Mode))
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Ref(EvidenceRef),
    Mode(FailureMode),
}

/// The evidence partitioned into independent groups: items in one group share an
/// ancestor or a failure mode, directly or through other items. Each group is a list of
/// indices into `items`, sorted; groups are sorted by their first index.
#[must_use]
pub fn independence_groups(items: &[EvidenceItem]) -> Vec<Vec<usize>> {
    // Union-find over items; two items are joined when they share any key.
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..items.len()).collect();
    let mut first_with: BTreeMap<Key, usize> = BTreeMap::new();
    for (i, item) in items.iter().enumerate() {
        for key in roots(item) {
            match first_with.get(&key) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
                None => {
                    first_with.insert(key, i);
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..items.len() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(i);
    }
    groups.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    use crate::digest::ContentDigest;
    use crate::ids::{ObservationId, ObserverId};
    use crate::observer::ObserverClass;
    use crate::testkit::{default_method, item};

    fn obs(observer: ObserverId) -> EvidenceItem {
        EvidenceItem::new(
            EvidenceRef::Observation(ObservationId::new()),
            ObserverClass::DirectSensor,
            observer,
            default_method(ObserverClass::DirectSensor),
        )
        .unwrap()
    }

    #[test]
    fn four_dashboards_on_one_trace_are_one_source() {
        let trace = EvidenceRef::Observation(ObservationId::new());
        let items: Vec<_> = (0..4)
            .map(|_| item(ObserverClass::ExternalSystem).derived_from(trace))
            .collect();
        assert_eq!(independence_groups(&items).len(), 1);
    }

    #[test]
    fn sdk_ebpf_and_database_views_are_independent_unless_they_share_a_mode() {
        let sdk = obs(ObserverId::new());
        let ebpf = obs(ObserverId::new());
        let db = obs(ObserverId::new());
        assert_eq!(
            independence_groups(&[sdk.clone(), ebpf.clone(), db.clone()]).len(),
            3
        );
        // The SDK and eBPF run on the same host and kernel boot: they fail together.
        let kernel = FailureMode::Kernel {
            host: "h1".into(),
            boot: "b1".into(),
        };
        let sdk = sdk.with_failure_mode(kernel.clone());
        let ebpf = ebpf.with_failure_mode(kernel);
        assert_eq!(independence_groups(&[sdk, ebpf, db]).len(), 2);
    }

    #[test]
    fn two_readings_by_one_model_and_prompt_are_one_source() {
        let model = FailureMode::Model(ContentDigest::of_bytes(b"model+prompt"));
        let a = item(ObserverClass::ModelExtractor).with_failure_mode(model.clone());
        let b = item(ObserverClass::ModelExtractor).with_failure_mode(model);
        assert_eq!(independence_groups(&[a, b]).len(), 1);
    }

    #[test]
    fn dependence_is_transitive_through_a_middle_item() {
        let shared_ab = FailureMode::HostClock("h1".into());
        let shared_bc = FailureMode::Upstream("export-7".into());
        let a = obs(ObserverId::new()).with_failure_mode(shared_ab.clone());
        let b = obs(ObserverId::new())
            .with_failure_mode(shared_ab)
            .with_failure_mode(shared_bc.clone());
        let c = obs(ObserverId::new()).with_failure_mode(shared_bc);
        assert_eq!(independence_groups(&[a, b, c]).len(), 1);
    }

    proptest! {
        // Groups are a partition: every item appears exactly once, and adding a shared
        // failure mode to all items always collapses them into one group.
        #[test]
        fn groups_partition_items(n in 1usize..20, shared in any::<bool>()) {
            let mut items: Vec<_> = (0..n).map(|_| obs(ObserverId::new())).collect();
            if shared {
                items = items.into_iter().map(|i| i.with_failure_mode(FailureMode::Upstream("u".into()))).collect();
            }
            let groups = independence_groups(&items);
            let mut seen: Vec<usize> = groups.iter().flatten().copied().collect();
            seen.sort_unstable();
            prop_assert_eq!(seen, (0..n).collect::<Vec<_>>());
            prop_assert_eq!(groups.len(), if shared { 1 } else { n });
        }
    }
}
