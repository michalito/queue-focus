//! Task placement, independent of GTK. Both pages read the same way — the hero
//! holding the current task, then Side, Next and Later — and differ only in
//! how those are laid out. Now is never a list: the one task in it is the hero's.
use super::Page;
use qf_core::{Bucket, Store};

pub(super) const ORDER: [Bucket; 4] = [Bucket::Now, Bucket::Side, Bucket::Next, Bucket::Later];

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

    /// The tasks shown for a bucket, in order; for Now, the one in the hero.
    pub fn rows(&self, bucket: Bucket) -> &[u64] {
        &self.buckets[ORDER.iter().position(|b| *b == bucket).unwrap()]
    }

    pub fn visible(&self, page: Page, later_open: bool) -> Vec<u64> {
        if page == Page::Settings {
            return Vec::new();
        }
        // The board gives Later a column of its own, so it is never collapsed.
        let later_open = later_open || page == Page::Board;
        ORDER
            .into_iter()
            .filter(|bucket| *bucket != Bucket::Later || later_open)
            .flat_map(|bucket| self.rows(bucket).iter().copied())
            .collect()
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
    /// The hero, or the Now heading: take over as the current task.
    Current,
    Append(Bucket),
    List {
        bucket: Bucket,
        before: Option<u64>,
    },
}

impl Destination {
    pub fn apply(self, store: &mut Store, id: u64) -> bool {
        match self {
            Self::Current => store.promote(id),
            Self::Append(bucket) => store.move_to(id, bucket, None),
            Self::List { bucket, before } => {
                let index = if let Some(anchor) = before {
                    // A stale row must not turn into a different destination.
                    if !Placement::new(store).rows(bucket).contains(&anchor) {
                        return false;
                    }
                    if anchor == id {
                        return store.get(id).is_some();
                    }
                    store
                        .in_bucket(bucket)
                        .filter(|t| t.id != id)
                        .position(|t| t.id == anchor)
                } else {
                    None
                };
                store.move_to(id, bucket, index)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buckets that are shown as lists, and so can be dragged within.
    const LISTED: [Bucket; 3] = [Bucket::Side, Bucket::Next, Bucket::Later];

    fn in_bucket(store: &Store, bucket: Bucket) -> Vec<u64> {
        store.in_bucket(bucket).map(|t| t.id).collect()
    }
    fn drop_before(store: &mut Store, id: u64, bucket: Bucket, before: Option<u64>) -> bool {
        Destination::List { bucket, before }.apply(store, id)
    }

    #[test]
    fn both_pages_read_hero_side_next_later() {
        let mut store = Store::new();
        let next = store.add("next", Bucket::Next, None);
        let later = store.add("later", Bucket::Later, None);
        let side = store.add("side", Bucket::Side, None);
        let current = store.add("current", Bucket::Now, None);
        let p = Placement::new(&store);
        assert_eq!(p.rows(Bucket::Now), [current]);
        assert_eq!(p.rows(Bucket::Next), [next]);
        assert_eq!(p.visible(Page::Queue, false), [current, side, next]);
        assert_eq!(p.visible(Page::Queue, true), [current, side, next, later]);
        // Later has a column of its own on the board: never collapsed.
        assert_eq!(p.visible(Page::Board, false), p.visible(Page::Queue, true));
        assert!(p.visible(Page::Settings, true).is_empty());
        assert!(Placement::default().visible(Page::Queue, true).is_empty());
        // An empty Now leaves the hero blank and the lists as they were.
        let mut store = Store::new();
        let only = store.add("only", Bucket::Next, None);
        let p = Placement::new(&store);
        assert!(p.rows(Bucket::Now).is_empty());
        assert_eq!(p.visible(Page::Board, false), [only]);
    }

    /// Every way a drop can name Now does the same thing: the task takes over
    /// and the one it replaces leads Next.
    #[test]
    fn every_drop_into_now_takes_over_as_the_current_task() {
        for destination in [
            Destination::Current,
            Destination::Append(Bucket::Now),
            Destination::List {
                bucket: Bucket::Now,
                before: None,
            },
        ] {
            let mut store = Store::new();
            let current = store.add("current", Bucket::Now, None);
            let next = store.add("next", Bucket::Next, None);
            let side = store.add("side", Bucket::Side, None);
            assert!(destination.apply(&mut store, side));
            assert_eq!(in_bucket(&store, Bucket::Now), [side]);
            assert_eq!(in_bucket(&store, Bucket::Next), [current, next]);
            assert_eq!(
                Placement::new(&store).visible(Page::Queue, false),
                [side, current, next]
            );
            // The current task dropped on its own panel stays where it is.
            assert!(destination.apply(&mut store, side));
            assert_eq!(in_bucket(&store, Bucket::Now), [side]);
        }
        // The current task can be dragged out, which empties the hero.
        let mut store = Store::new();
        let current = store.add("current", Bucket::Now, None);
        let next = store.add("next", Bucket::Next, None);
        assert!(drop_before(&mut store, current, Bucket::Next, Some(next)));
        assert!(store.current().is_none());
        assert_eq!(in_bucket(&store, Bucket::Next), [current, next]);
    }

    /// Every drag a user can start, dropped everywhere it can land in a list.
    #[test]
    fn anchored_moves_cover_every_source_and_destination_pair() {
        let fixture = || {
            let mut store = Store::new();
            for bucket in LISTED {
                for _ in 0..3 {
                    store.add("task", bucket, None);
                }
            }
            store.add("current", Bucket::Now, None);
            store
        };
        for source in ORDER {
            for destination in LISTED {
                let rows = Placement::new(&fixture()).rows(source).len();
                // Now offers the one task in the hero.
                assert_eq!(rows, if source == Bucket::Now { 1 } else { 3 });
                for source_index in 0..rows {
                    for target_index in 0..=3 {
                        let mut store = fixture();
                        let p = Placement::new(&store);
                        let id = p.rows(source)[source_index];
                        let anchor = p.rows(destination).get(target_index).copied();
                        let mut expected = in_bucket(&store, destination);
                        if anchor != Some(id) {
                            expected.retain(|candidate| *candidate != id);
                            let index = anchor
                                .and_then(|a| expected.iter().position(|candidate| *candidate == a))
                                .unwrap_or(expected.len());
                            expected.insert(index, id);
                        }
                        assert!(drop_before(&mut store, id, destination, anchor));
                        assert_eq!(in_bucket(&store, destination), expected);
                        assert_eq!(store.tasks.len(), 10);
                        assert!(store.in_bucket(Bucket::Now).count() <= 1);
                    }
                }
            }
        }
    }

    #[test]
    fn missing_or_stale_anchors_are_rejected_without_moving_the_source() {
        let mut store = Store::new();
        let current = store.add("current", Bucket::Now, None);
        let source = store.add("source", Bucket::Later, None);
        // The current task is the hero's, not a row of Next; 999 is nobody's.
        for anchor in [current, 999] {
            assert!(!drop_before(&mut store, source, Bucket::Next, Some(anchor)));
            assert_eq!(store.get(source).unwrap().bucket, Bucket::Later);
        }
        assert!(!Destination::Append(Bucket::Later).apply(&mut store, 999));
        assert!(!Destination::Current.apply(&mut store, 999));
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
