//! Task placement, independent of GTK. A displayed section is not a bucket:
//! the Next section contains both the Now tail and the Next bucket, because the
//! head of Now belongs to the hero panel rather than to any list. Both pages
//! read the same way — hero, Side, Now tail, Next, Later — and differ only in
//! how those are laid out.
use super::Page;
use qf_core::{Bucket, Store};

pub(super) const ORDER: [Bucket; 4] = [Bucket::Now, Bucket::Side, Bucket::Next, Bucket::Later];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct List {
    pub page: Page,
    pub bucket: Bucket,
}

impl List {
    /// The header this list is shown under. Now has no list of its own: its
    /// tail queues under Next, behind whatever the hero is showing.
    pub fn section(self) -> Section {
        Section {
            page: self.page,
            bucket: match self.bucket {
                Bucket::Now => Bucket::Next,
                bucket => bucket,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Section {
    pub page: Page,
    pub bucket: Bucket,
}

impl Section {
    /// The lists under this header, in order. A Now section has none: the hero
    /// shows the head, and everything behind it is listed under Next.
    pub fn lists(self) -> Vec<List> {
        let buckets = match self.bucket {
            Bucket::Now => vec![],
            Bucket::Next => vec![Bucket::Now, Bucket::Next],
            bucket => vec![bucket],
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
        if list.bucket == Bucket::Now {
            // The head is the hero's, so it is nobody's row.
            &ids[ids.len().min(1)..]
        } else {
            ids
        }
    }

    pub fn count(&self, section: Section) -> usize {
        if section.bucket == Bucket::Now {
            // The hero holds one task at a time; the rest counts under Next.
            return self.buckets[0].len().min(1);
        }
        section
            .lists()
            .into_iter()
            .map(|list| self.rows(list).len())
            .sum()
    }

    /// Whether this list leads its section — the one row under a header that
    /// draws no hairline above it.
    pub fn leads(&self, list: List) -> bool {
        list.section()
            .lists()
            .into_iter()
            .take_while(|candidate| *candidate != list)
            .all(|earlier| self.rows(earlier).is_empty())
    }

    pub fn visible(&self, page: Page, later_open: bool) -> Vec<u64> {
        if page == Page::Settings {
            return Vec::new();
        }
        // The board gives Later a column of its own, so it is never collapsed.
        let later_open = later_open || page == Page::Board;
        self.buckets[0]
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
    fn section(page: Page, bucket: Bucket) -> Section {
        Section { page, bucket }
    }
    fn in_bucket(store: &Store, bucket: Bucket) -> Vec<u64> {
        store.in_bucket(bucket).map(|t| t.id).collect()
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
        for page in [Page::Queue, Page::Board] {
            // The head is the hero's on either page, so the tail is the list.
            assert_eq!(p.rows(list(page, Bucket::Now)), [tail]);
            assert!(section(page, Bucket::Now).lists().is_empty());
            // Now counts the one task on show; the tail counts under Next.
            assert_eq!(p.count(section(page, Bucket::Now)), 1);
            assert_eq!(p.count(section(page, Bucket::Next)), 2);
            // The tail leads the Next section; Next's own list follows it.
            assert!(p.leads(list(page, Bucket::Now)));
            assert!(!p.leads(list(page, Bucket::Next)));
            assert!(p.leads(list(page, Bucket::Side)));
        }
        assert_eq!(p.visible(Page::Queue, false), [current, side, tail, next]);
        assert_eq!(
            p.visible(Page::Queue, true),
            [current, side, tail, next, later]
        );
        // Later has a column of its own on the board: never collapsed.
        assert_eq!(
            p.visible(Page::Board, false),
            [current, side, tail, next, later]
        );
        assert_eq!(p.visible(Page::Board, false), p.visible(Page::Queue, true));
        assert!(p.visible(Page::Settings, true).is_empty());
        assert!(Placement::default().visible(Page::Queue, true).is_empty());
        // An empty Now leaves the hero blank and Next leading its own section.
        let mut store = Store::new();
        let only = store.add("only", Bucket::Next, None, false);
        let p = Placement::new(&store);
        assert_eq!(p.count(section(Page::Board, Bucket::Now)), 0);
        assert!(p.rows(list(Page::Board, Bucket::Now)).is_empty());
        assert!(p.leads(list(Page::Board, Bucket::Next)));
        assert_eq!(p.visible(Page::Board, false), [only]);
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
            [next, tail]
        );
        assert!(Destination::Append(Bucket::Next).apply(&mut store, tail));
        assert_eq!(store.get(tail).unwrap().bucket, Bucket::Next);
        // J/K stays within its stored bucket despite the composite section.
        assert!(!store.shift(tail, -1));
        assert!(Destination::Banner.apply(&mut store, tail));
        assert_eq!(store.current().unwrap().id, tail);
        // The head is not a row, so no drop can anchor on it — taking over as
        // the current task means dropping on the hero itself.
        assert!(!drop_before(
            &mut store,
            current,
            list(Page::Board, Bucket::Now),
            Some(tail)
        ));
        assert_eq!(store.current().unwrap().id, tail);
        assert!(Destination::Banner.apply(&mut store, current));
        assert_eq!(store.current().unwrap().id, current);
    }

    /// Every drag a user can start, dropped everywhere it can land. The rows
    /// are what can be picked up and aimed at; the expectation is modelled in
    /// the stored bucket, which is where the move actually has to land.
    #[test]
    fn anchored_moves_cover_every_source_and_destination_pair() {
        let fixture = || {
            let mut store = Store::new();
            for bucket in ORDER {
                for _ in 0..3 {
                    store.add("task", bucket, None, false);
                }
            }
            store
        };
        for page in [Page::Queue, Page::Board] {
            for source in ORDER {
                for destination in ORDER {
                    let rows = Placement::new(&fixture()).rows(list(page, source)).len();
                    let targets = Placement::new(&fixture())
                        .rows(list(page, destination))
                        .len();
                    // Now offers one row fewer: its head is the hero's.
                    assert_eq!(rows, if source == Bucket::Now { 2 } else { 3 });
                    for source_index in 0..rows {
                        for target_index in 0..=targets {
                            let mut store = fixture();
                            let p = Placement::new(&store);
                            let id = p.rows(list(page, source))[source_index];
                            let region = list(page, destination);
                            let anchor = p.rows(region).get(target_index).copied();
                            let mut expected = in_bucket(&store, destination);
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
                            assert_eq!(in_bucket(&store, destination), expected);
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
        for page in [Page::Queue, Page::Board] {
            for anchor in [current, 999] {
                assert!(!drop_before(
                    &mut store,
                    source,
                    list(page, Bucket::Now),
                    Some(anchor)
                ));
                assert_eq!(store.get(source).unwrap().bucket, Bucket::Next);
            }
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
