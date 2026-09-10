//! Task placement, independent of GTK. A displayed section is not a bucket:
//! Queue's Next section contains both the Now tail and the Next bucket.
use super::Page;
use qf_core::{Bucket, Store};

pub(super) const ORDER: [Bucket; 4] = [Bucket::Now, Bucket::Side, Bucket::Next, Bucket::Later];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct List {
    pub page: Page,
    pub bucket: Bucket,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Section {
    pub page: Page,
    pub bucket: Bucket,
}

impl Section {
    pub fn lists(self) -> Vec<List> {
        let buckets = if self.page == Page::Queue && self.bucket == Bucket::Next {
            vec![Bucket::Now, Bucket::Next]
        } else {
            vec![self.bucket]
        };
        buckets
            .into_iter()
            .map(|bucket| List {
                page: self.page,
                bucket,
            })
            .collect()
    }
}

/// Owned snapshot of what was rendered, retained across store mutations.
#[derive(Default)]
pub(super) struct Placement {
    buckets: [Vec<u64>; 4],
}

impl Placement {
    pub fn new(store: &Store) -> Self {
        Self {
            buckets: ORDER.map(|bucket| store.in_bucket(bucket).map(|t| t.id).collect()),
        }
    }

    pub fn rows(&self, list: List) -> &[u64] {
        let ids = &self.buckets[ORDER.iter().position(|b| *b == list.bucket).unwrap()];
        if list.page == Page::Queue && list.bucket == Bucket::Now {
            &ids[ids.len().min(1)..]
        } else {
            ids
        }
    }

    pub fn count(&self, section: Section) -> usize {
        section
            .lists()
            .into_iter()
            .map(|list| self.rows(list).len())
            .sum()
    }

    pub fn visible(&self, page: Page, later_open: bool) -> Vec<u64> {
        match page {
            Page::Settings => Vec::new(),
            Page::Board => self.buckets.iter().flatten().copied().collect(),
            Page::Queue => self.buckets[0]
                .first()
                .into_iter()
                .copied()
                .chain(
                    [Bucket::Side, Bucket::Next, Bucket::Later]
                        .into_iter()
                        .filter(|bucket| *bucket != Bucket::Later || later_open)
                        .flat_map(|bucket| Section { page, bucket }.lists())
                        .flat_map(|list| self.rows(list).iter().copied()),
                )
                .collect(),
        }
    }
}

pub(super) struct Focus {
    id: u64,
    index: usize,
}

impl Focus {
    pub fn capture(order: &[u64], id: u64) -> Option<Self> {
        Some(Self {
            id,
            index: order.iter().position(|candidate| *candidate == id)?,
        })
    }

    pub fn restore(self, order: &[u64]) -> Option<u64> {
        if order.contains(&self.id) {
            Some(self.id)
        } else {
            order
                .get(self.index.min(order.len().saturating_sub(1)))
                .copied()
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Destination {
    Banner,
    Append(Bucket),
    List { list: List, before: Option<u64> },
}

impl Destination {
    pub fn apply(self, store: &mut Store, id: u64) -> bool {
        match self {
            Self::Banner => store.promote(id),
            Self::Append(bucket) => store.move_to(id, bucket, None),
            Self::List { list, before } => {
                let index = if let Some(anchor) = before {
                    // A stale row must not turn into a different destination.
                    if !Placement::new(store).rows(list).contains(&anchor) {
                        return false;
                    }
                    if anchor == id {
                        return store.get(id).is_some();
                    }
                    store
                        .in_bucket(list.bucket)
                        .filter(|t| t.id != id)
                        .position(|t| t.id == anchor)
                } else {
                    None
                };
                store.move_to(id, list.bucket, index)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(page: Page, bucket: Bucket) -> List {
        List { page, bucket }
    }
    fn drop_before(store: &mut Store, id: u64, list: List, before: Option<u64>) -> bool {
        Destination::List { list, before }.apply(store, id)
    }

    #[test]
    fn placement_counts_and_navigation_share_the_composite_section() {
        let mut store = Store::new();
        let current = store.add("current", Bucket::Now, None, false);
        let tail = store.add("tail", Bucket::Now, None, false);
        let next = store.add("next", Bucket::Next, None, false);
        let side = store.add("side", Bucket::Side, None, false);
        let later = store.add("later", Bucket::Later, None, false);
        let p = Placement::new(&store);
        assert_eq!(p.rows(list(Page::Queue, Bucket::Now)), [tail]);
        assert_eq!(
            p.count(Section {
                page: Page::Queue,
                bucket: Bucket::Next
            }),
            2
        );
        assert_eq!(p.visible(Page::Queue, false), [current, side, tail, next]);
        assert_eq!(
            p.visible(Page::Queue, true),
            [current, side, tail, next, later]
        );
        assert_eq!(
            p.visible(Page::Board, false),
            [current, tail, side, next, later]
        );
        assert!(p.visible(Page::Settings, true).is_empty());
        assert!(Placement::default().visible(Page::Queue, true).is_empty());
    }

    #[test]
    fn moves_use_actual_buckets_and_refresh_both_pages() {
        let mut store = Store::new();
        let current = store.add("current", Bucket::Now, None, false);
        let tail = store.add("tail", Bucket::Now, None, false);
        let next = store.add("next", Bucket::Next, None, false);
        assert!(drop_before(
            &mut store,
            next,
            list(Page::Queue, Bucket::Now),
            Some(tail)
        ));
        assert_eq!(
            Placement::new(&store).visible(Page::Queue, false),
            [current, next, tail]
        );
        assert_eq!(
            Placement::new(&store).rows(list(Page::Board, Bucket::Now)),
            [current, next, tail]
        );
        assert!(Destination::Append(Bucket::Next).apply(&mut store, tail));
        assert_eq!(store.get(tail).unwrap().bucket, Bucket::Next);
        // J/K stays within its stored bucket despite the composite section.
        assert!(!store.shift(tail, -1));
        assert!(Destination::Banner.apply(&mut store, tail));
        assert_eq!(store.current().unwrap().id, tail);
        assert!(drop_before(
            &mut store,
            current,
            list(Page::Board, Bucket::Now),
            Some(tail)
        ));
        assert_eq!(store.current().unwrap().id, current);
    }

    #[test]
    fn anchored_moves_cover_every_source_and_destination_pair() {
        for page in [Page::Queue, Page::Board] {
            for source in ORDER {
                for destination in ORDER {
                    for source_index in 0..3 {
                        for target_index in 0..=3 {
                            let mut store = Store::new();
                            for bucket in ORDER {
                                for _ in 0..3 {
                                    store.add("task", bucket, None, false);
                                }
                            }
                            let p = Placement::new(&store);
                            let id = p.rows(list(Page::Board, source))[source_index];
                            let region = list(page, destination);
                            let anchor = p.rows(region).get(target_index).copied();
                            let mut expected = p.rows(list(Page::Board, destination)).to_vec();
                            if anchor != Some(id) {
                                expected.retain(|candidate| *candidate != id);
                                let index = anchor
                                    .and_then(|a| {
                                        expected.iter().position(|candidate| *candidate == a)
                                    })
                                    .unwrap_or(expected.len());
                                expected.insert(index, id);
                            }
                            assert!(drop_before(&mut store, id, region, anchor));
                            assert_eq!(
                                Placement::new(&store).rows(list(Page::Board, destination)),
                                expected
                            );
                            assert_eq!(store.tasks.len(), 12);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn missing_or_stale_anchors_are_rejected_without_moving_the_source() {
        let mut store = Store::new();
        let current = store.add("current", Bucket::Now, None, false);
        let source = store.add("source", Bucket::Next, None, false);
        for anchor in [current, 999] {
            assert!(!drop_before(
                &mut store,
                source,
                list(Page::Queue, Bucket::Now),
                Some(anchor)
            ));
            assert_eq!(store.get(source).unwrap().bucket, Bucket::Next);
        }
        assert!(!Destination::Append(Bucket::Later).apply(&mut store, 999));
    }

    #[test]
    fn focus_uses_previous_position_when_a_task_leaves_the_visible_order() {
        let old = [1, 2, 3, 4];
        assert_eq!(
            Focus::capture(&old, 3).unwrap().restore(&[3, 1, 2, 4]),
            Some(3)
        );
        assert_eq!(
            Focus::capture(&old, 3).unwrap().restore(&[1, 2, 4]),
            Some(4)
        );
        assert_eq!(Focus::capture(&old, 4).unwrap().restore(&[1, 2]), Some(2));
        assert_eq!(Focus::capture(&old, 1).unwrap().restore(&[]), None);
        assert!(Focus::capture(&old, 99).is_none());
    }
}
