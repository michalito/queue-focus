use serde::{Deserialize, Deserializer, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Task titles are short, single-purpose labels, not documents. Keeping this
/// invariant in the model bounds every snapshot and renderer fed by the store.
pub const MAX_TITLE_CHARS: usize = 256;

/// Quick-add includes optional bucket/tag markers as well as the title. Bound
/// it before splitting so an IPC caller cannot make parsing allocate an
/// unbounded word vector just to produce a bounded title.
pub const MAX_QUICK_ADD_BYTES: usize = 4096;

fn bounded_title(title: &str) -> String {
    title.trim().chars().take(MAX_TITLE_CHARS).collect()
}

fn deserialize_title<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    String::deserialize(deserializer).map(|title| bounded_title(&title))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    Now,
    Next,
    Later,
    Side,
}

impl Bucket {
    pub const ALL: [Bucket; 4] = [Bucket::Now, Bucket::Next, Bucket::Later, Bucket::Side];

    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Now => "now",
            Bucket::Next => "next",
            Bucket::Later => "later",
            Bucket::Side => "side",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Bucket::Now => "Now",
            Bucket::Next => "Next",
            Bucket::Later => "Later",
            Bucket::Side => "Side",
        }
    }

    pub fn parse(s: &str) -> Option<Bucket> {
        match s.trim().to_ascii_lowercase().as_str() {
            "now" | "n" => Some(Bucket::Now),
            "next" | "x" => Some(Bucket::Next),
            "later" | "l" => Some(Bucket::Later),
            "side" | "s" => Some(Bucket::Side),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tag {
    Work,
    Personal,
}

impl Tag {
    pub fn as_str(self) -> &'static str {
        match self {
            Tag::Work => "work",
            Tag::Personal => "personal",
        }
    }

    pub fn parse(s: &str) -> Option<Tag> {
        match s.trim().to_ascii_lowercase().as_str() {
            "work" | "w" => Some(Tag::Work),
            "personal" | "p" => Some(Tag::Personal),
            _ => None,
        }
    }

    /// none -> work -> personal -> none
    pub fn cycle(cur: Option<Tag>) -> Option<Tag> {
        match cur {
            None => Some(Tag::Work),
            Some(Tag::Work) => Some(Tag::Personal),
            Some(Tag::Personal) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    #[serde(deserialize_with = "deserialize_title")]
    pub title: String,
    pub bucket: Bucket,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<Tag>,
    pub created_at: u64,
    /// Unix seconds since this task became the current task, the one in Now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Unix seconds at which the timer was paused; `None` while it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at: Option<u64>,
}

impl Task {
    /// Time on the clock: frozen at `paused_at` while paused.
    pub fn elapsed_secs(&self, now: u64) -> Option<u64> {
        let started = self.started_at?;
        Some(self.paused_at.unwrap_or(now).saturating_sub(started))
    }

    pub fn is_paused(&self) -> bool {
        self.started_at.is_some() && self.paused_at.is_some()
    }
}

/// Result of parsing quick-add syntax:
/// `!title` or a bare `!now` -> Now, `title #work` / `#w` / `#p`,
/// `@later` / `@side` / `@next` / `@now`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAdd {
    pub title: String,
    pub bucket: Option<Bucket>,
    pub tag: Option<Tag>,
}

impl QuickAdd {
    pub fn parse(input: &str) -> Option<QuickAdd> {
        if input.len() > MAX_QUICK_ADD_BYTES {
            return None;
        }
        let mut bucket = None;
        let mut tag = None;
        let mut words: Vec<&str> = Vec::new();
        for w in input.split_whitespace() {
            if let Some(t) = w.strip_prefix('#').and_then(Tag::parse) {
                tag = Some(t);
            } else if let Some(b) = w.strip_prefix('@').and_then(Bucket::parse) {
                bucket = Some(b);
            } else if is_now_marker(w) {
                // The entry's placeholder advertises "!now" as a word of its own.
                bucket = Some(Bucket::Now);
            } else {
                words.push(w);
            }
        }
        let mut title = words.join(" ");
        if let Some(rest) = title.strip_prefix('!') {
            bucket = Some(Bucket::Now);
            title = rest.trim_start().to_string();
        }
        if title.is_empty() {
            return None;
        }
        Some(QuickAdd { title, bucket, tag })
    }
}

/// A standalone "!" / "!now" / "!n" word: shorthand for the Now bucket.
fn is_now_marker(word: &str) -> bool {
    match word.strip_prefix('!') {
        Some("") => true,
        Some(rest) => Bucket::parse(rest) == Some(Bucket::Now),
        None => false,
    }
}

/// What `Store::complete` removed, where it sat, and what it pulled into Now
/// in its place, so the completion can be reversed exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    pub task: Task,
    /// Position of `task` in `tasks` before it was removed.
    pub index: usize,
    /// The task moved from the head of Next into Now, if any.
    pub pulled: Option<u64>,
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ordered task store. `tasks` order is the display order within each bucket.
///
/// Now is a slot rather than a list: it holds the one task being done, the
/// current task. A task that enters Now takes the slot, and the task it found
/// there steps back to the front of Next, where completing pulls from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredTasks")]
pub struct Store {
    pub next_id: u64,
    pub tasks: Vec<Task>,
}

/// The task file as it was written. Older versions queued tasks up in Now, so
/// what is read goes through `normalize` like every other way into the model.
#[derive(Deserialize)]
struct StoredTasks {
    #[serde(default = "one")]
    next_id: u64,
    #[serde(default)]
    tasks: Vec<Task>,
}

impl From<StoredTasks> for Store {
    fn from(stored: StoredTasks) -> Self {
        let mut store = Store {
            next_id: stored.next_id,
            tasks: stored.tasks,
        };
        store.normalize();
        store
    }
}

fn one() -> u64 {
    1
}

impl Store {
    pub fn new() -> Self {
        Store {
            next_id: 1,
            tasks: Vec::new(),
        }
    }

    // ---- queries -------------------------------------------------------

    pub fn get(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn in_bucket(&self, bucket: Bucket) -> impl Iterator<Item = &Task> {
        self.tasks.iter().filter(move |t| t.bucket == bucket)
    }

    /// The task being done: the one in Now.
    pub fn current(&self) -> Option<&Task> {
        self.in_bucket(Bucket::Now).next()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    // ---- mutations (all call normalize) -------------------------------

    /// Add a task at the end of `bucket`. Added to Now, it becomes current.
    pub fn add(&mut self, title: &str, bucket: Bucket, tag: Option<Tag>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let task = Task {
            id,
            title: bounded_title(title),
            bucket,
            tag,
            created_at: unix_now(),
            started_at: None,
            paused_at: None,
        };
        self.tasks.push(task);
        if bucket == Bucket::Now {
            self.place(id, Bucket::Now, Some(0));
        }
        self.normalize();
        id
    }

    pub fn quick_add(&mut self, input: &str, default_bucket: Bucket) -> Option<u64> {
        let q = QuickAdd::parse(input)?;
        let bucket = q.bucket.unwrap_or(default_bucket);
        Some(self.add(&q.title, bucket, q.tag))
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        let removed = self.tasks.len() != before;
        if removed {
            self.normalize();
        }
        removed
    }

    /// Delete the current task and pull the head of Next into Now.
    pub fn complete_current(&mut self) -> Option<Completed> {
        let id = self.current()?.id;
        self.complete(id)
    }

    /// Mark a task done: delete it, and when it was the current task, pull the
    /// head of Next into Now. Any other task is simply deleted.
    pub fn complete(&mut self, id: u64) -> Option<Completed> {
        let index = self.tasks.iter().position(|t| t.id == id)?;
        let task = self.tasks.remove(index);
        let mut pulled = None;
        if task.bucket == Bucket::Now {
            pulled = self.in_bucket(Bucket::Next).next().map(|t| t.id);
            if let Some(next) = pulled {
                self.place(next, Bucket::Now, Some(0));
            }
        }
        self.normalize();
        Some(Completed {
            task,
            index,
            pulled,
        })
    }

    /// Reverse `complete`: the pulled task goes back to the head of Next and
    /// the completed task returns to where it was, so a completed current
    /// task is current again with its clock intact. Refuses if a task with
    /// that id already exists.
    pub fn undo_complete(&mut self, completed: Completed) -> bool {
        if self.get(completed.task.id).is_some() {
            return false;
        }
        if let Some(pulled) = completed.pulled {
            if self.get(pulled).is_some_and(|t| t.bucket == Bucket::Now) {
                self.place(pulled, Bucket::Next, Some(0));
            }
        }
        let at = completed.index.min(self.tasks.len());
        self.tasks.insert(at, completed.task);
        self.normalize();
        true
    }

    /// Make a task the current one. The task it replaces steps back to the
    /// front of Next.
    pub fn promote(&mut self, id: u64) -> bool {
        self.move_to(id, Bucket::Now, None)
    }

    /// Move a task into `bucket` at `index` (None = end). Now has one place
    /// in it, so moving a task there promotes it whatever the index.
    pub fn move_to(&mut self, id: u64, bucket: Bucket, index: Option<usize>) -> bool {
        if bucket == Bucket::Now && self.current().is_some_and(|t| t.id == id) {
            // Already there: leave the store as it is, so nothing is saved.
            return true;
        }
        let index = if bucket == Bucket::Now {
            Some(0)
        } else {
            index
        };
        let moved = self.place(id, bucket, index);
        if moved {
            self.normalize();
        }
        moved
    }

    /// Reposition a task without settling the store; callers normalize.
    fn place(&mut self, id: u64, bucket: Bucket, index: Option<usize>) -> bool {
        let Some(pos) = self.tasks.iter().position(|t| t.id == id) else {
            return false;
        };
        let mut task = self.tasks.remove(pos);
        task.bucket = bucket;
        let ids: Vec<u64> = self.in_bucket(bucket).map(|t| t.id).collect();
        let insert_at = match index {
            Some(i) if i < ids.len() => self.tasks.iter().position(|t| t.id == ids[i]).unwrap(),
            _ => self
                .tasks
                .iter()
                .rposition(|t| t.bucket == bucket)
                .map(|p| p + 1)
                .unwrap_or(self.tasks.len()),
        };
        self.tasks.insert(insert_at, task);
        true
    }

    /// Move a task up (-1) or down (+1) within its bucket.
    pub fn shift(&mut self, id: u64, delta: i32) -> bool {
        let Some(task) = self.get(id) else {
            return false;
        };
        let bucket = task.bucket;
        let ids: Vec<u64> = self.in_bucket(bucket).map(|t| t.id).collect();
        let i = ids.iter().position(|&x| x == id).unwrap();
        let target = (i as i64 + delta as i64).clamp(0, ids.len() as i64 - 1) as usize;
        if target == i {
            return false;
        }
        self.move_to(id, bucket, Some(target))
    }

    pub fn set_tag(&mut self, id: u64, tag: Option<Tag>) -> bool {
        match self.tasks.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.tag = tag;
                true
            }
            None => false,
        }
    }

    pub fn cycle_tag(&mut self, id: u64) -> bool {
        let cur = match self.get(id) {
            Some(t) => t.tag,
            None => return false,
        };
        self.set_tag(id, Tag::cycle(cur))
    }

    /// Pause or resume the current task's timer. Resuming keeps the time already
    /// on the clock by shifting `started_at` forward by the paused duration.
    pub fn toggle_pause(&mut self) -> bool {
        let Some(id) = self.current().map(|t| t.id) else {
            return false;
        };
        let now = unix_now();
        let Some(task) = self.tasks.iter_mut().find(|t| t.id == id) else {
            return false;
        };
        let Some(started) = task.started_at else {
            return false;
        };
        match task.paused_at.take() {
            Some(paused) => task.started_at = Some(started + now.saturating_sub(paused)),
            None => task.paused_at = Some(now.max(started)),
        }
        true
    }

    pub fn rename(&mut self, id: u64, title: &str) -> bool {
        let title = bounded_title(title);
        if title.is_empty() {
            return false;
        }
        match self.tasks.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.title = title;
                true
            }
            None => false,
        }
    }

    /// Settle the store after a change. Now keeps its first task and the rest
    /// step back to the front of Next, in order; then only the current task
    /// carries a (possibly paused) timer.
    fn normalize(&mut self) {
        let displaced: Vec<u64> = self.in_bucket(Bucket::Now).skip(1).map(|t| t.id).collect();
        for id in displaced.into_iter().rev() {
            self.place(id, Bucket::Next, Some(0));
        }
        let cur = self.current().map(|t| t.id);
        let now = unix_now();
        for t in &mut self.tasks {
            if Some(t.id) == cur {
                if t.started_at.is_none() {
                    t.started_at = Some(now);
                    t.paused_at = None;
                }
            } else {
                t.started_at = None;
                t.paused_at = None;
            }
        }
    }

    /// Compact JSON snapshot for the shell extension / CLI.
    pub fn snapshot_json(&self) -> String {
        let task = |t: &Task| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "tag": t.tag.map(Tag::as_str),
                "started_at": t.started_at,
                "paused_at": t.paused_at,
            })
        };
        let list = |b: Bucket| self.in_bucket(b).map(task).collect::<Vec<_>>();
        serde_json::json!({
            "current": self.current().map(task),
            "now": list(Bucket::Now),
            "side": list(Bucket::Side),
            "next": list(Bucket::Next),
            "later": list(Bucket::Later),
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(s: &Store, b: Bucket) -> Vec<u64> {
        s.in_bucket(b).map(|t| t.id).collect()
    }

    #[test]
    fn add_and_order() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Next, None);
        let b = s.add("b", Bucket::Next, None);
        let c = s.add("c", Bucket::Next, None);
        assert_eq!(ids(&s, Bucket::Next), vec![a, b, c]);
        assert!(s.current().is_none());
    }

    #[test]
    fn promote_and_timer() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Next, None);
        let b = s.add("b", Bucket::Later, None);
        assert!(s.promote(b));
        assert_eq!(s.current().unwrap().id, b);
        assert!(s.get(b).unwrap().started_at.is_some());
        assert!(s.get(a).unwrap().started_at.is_none());
        s.promote(a);
        assert_eq!(ids(&s, Bucket::Now), vec![a]);
        assert!(s.get(b).unwrap().started_at.is_none(), "only Now is timed");
    }

    /// Every way into Now, with a task already there.
    #[test]
    fn now_holds_one_task_and_the_one_it_replaces_steps_back_to_next() {
        /// Puts a task into Now: the listed one, or one of its own making.
        type Enter = fn(&mut Store, u64);
        let ways: [(&str, Enter); 5] = [
            ("promote", |s, id| assert!(s.promote(id))),
            ("move to the end", |s, id| {
                assert!(s.move_to(id, Bucket::Now, None))
            }),
            ("move to an index", |s, id| {
                assert!(s.move_to(id, Bucket::Now, Some(7)))
            }),
            ("add", |s, _| {
                s.add("added", Bucket::Now, None);
            }),
            ("quick add", |s, _| {
                s.quick_add("!added", Bucket::Later).unwrap();
            }),
        ];
        for (way, enter) in ways {
            let mut s = Store::new();
            let was = s.add("was current", Bucket::Now, None);
            let queued = s.add("queued", Bucket::Next, None);
            let side = s.add("side", Bucket::Side, None);
            s.tasks[0].started_at = Some(1000);
            assert!(s.toggle_pause());

            enter(&mut s, side);

            assert_eq!(s.in_bucket(Bucket::Now).count(), 1, "{way}");
            assert_ne!(s.current().unwrap().id, was, "{way}");
            assert!(s.current().unwrap().started_at.is_some(), "{way}");
            assert_eq!(
                ids(&s, Bucket::Next),
                vec![was, queued],
                "{way}: completing pulls it straight back"
            );
            let was = s.get(was).unwrap();
            assert!(was.started_at.is_none() && !was.is_paused(), "{way}");
        }
    }

    #[test]
    fn entering_now_again_changes_nothing() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, None);
        let b = s.add("b", Bucket::Next, None);
        s.tasks[0].started_at = Some(1000);
        let before = s.clone();
        assert!(s.promote(a));
        assert!(s.move_to(a, Bucket::Now, Some(3)));
        assert!(!s.shift(a, 1), "there is nowhere to go within Now");
        assert_eq!(s, before);
        assert_eq!(ids(&s, Bucket::Next), vec![b]);
    }

    /// Older versions let tasks queue up in Now, behind the current one.
    #[test]
    fn a_file_with_several_tasks_in_now_keeps_the_first_and_queues_the_rest() {
        let json = r#"{"next_id":6,"tasks":[
            {"id":1,"title":"next","bucket":"next","created_at":0},
            {"id":2,"title":"current","bucket":"now","created_at":0,"started_at":10,"paused_at":40},
            {"id":3,"title":"side","bucket":"side","created_at":0},
            {"id":4,"title":"behind","bucket":"now","created_at":0},
            {"id":5,"title":"further behind","bucket":"now","created_at":0,"started_at":20}]}"#;
        let s: Store = serde_json::from_str(json).unwrap();
        assert_eq!(ids(&s, Bucket::Now), vec![2]);
        assert_eq!(ids(&s, Bucket::Next), vec![4, 5, 1]);
        assert_eq!(ids(&s, Bucket::Side), vec![3]);
        let current = s.current().unwrap();
        assert_eq!(
            (current.started_at, current.paused_at),
            (Some(10), Some(40))
        );
        assert!(s.get(5).unwrap().started_at.is_none());
        assert_eq!(s.next_id, 6);

        // What is written back is already settled, so it loads unchanged.
        let again: Store = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(again, s);
    }

    #[test]
    fn complete_pulls_from_next() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, None);
        let b = s.add("b", Bucket::Next, None);
        let c = s.add("c", Bucket::Next, None);
        let done = s.complete_current().unwrap();
        assert_eq!(done.task.id, a);
        assert_eq!(done.pulled, Some(b));
        assert_eq!(s.current().unwrap().id, b);
        assert_eq!(ids(&s, Bucket::Next), vec![c]);
        assert_eq!(s.len(), 2, "done tasks are deleted, not kept");
        s.complete_current();
        assert_eq!(s.current().unwrap().id, c);
        s.complete_current();
        assert!(s.current().is_none());
        assert!(s.complete_current().is_none());
    }

    #[test]
    fn undo_complete_restores_the_task_and_returns_the_pulled_one() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, Some(Tag::Work));
        let b = s.add("b", Bucket::Next, None);
        let c = s.add("c", Bucket::Next, None);
        s.tasks[0].started_at = Some(1000);
        let done = s.complete_current().unwrap();
        assert!(s.undo_complete(done));
        assert_eq!(ids(&s, Bucket::Now), vec![a]);
        assert_eq!(ids(&s, Bucket::Next), vec![b, c]);
        let cur = s.current().unwrap();
        assert_eq!(cur.tag, Some(Tag::Work));
        assert_eq!(cur.started_at, Some(1000), "the clock carries on");
        assert!(s.get(b).unwrap().started_at.is_none());
    }

    #[test]
    fn undo_complete_without_a_pull_fills_now_again() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, None);
        let b = s.add("b", Bucket::Side, None);
        let done = s.complete_current().unwrap();
        assert_eq!(done.pulled, None, "nothing in Next to pull");
        assert!(s.current().is_none());
        assert!(s.undo_complete(done));
        assert_eq!(ids(&s, Bucket::Now), vec![a]);
        assert_eq!(ids(&s, Bucket::Side), vec![b]);
    }

    /// The pulled task sits after the completed one's old index or before it
    /// depending on where Next was stored; undo has to win the slot either way.
    #[test]
    fn undo_complete_takes_the_slot_back_wherever_the_pulled_task_is_stored() {
        let mut s = Store::new();
        let b = s.add("b", Bucket::Next, None);
        let a = s.add("a", Bucket::Now, None);
        let done = s.complete_current().unwrap();
        assert_eq!(s.current().unwrap().id, b);
        assert!(s.undo_complete(done));
        assert_eq!(ids(&s, Bucket::Now), vec![a]);
        assert_eq!(ids(&s, Bucket::Next), vec![b]);
    }

    #[test]
    fn complete_deletes_any_other_task_and_undo_puts_it_back_in_place() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, None);
        let b = s.add("b", Bucket::Side, None);
        let c = s.add("c", Bucket::Side, None);
        let d = s.add("d", Bucket::Side, None);
        let e = s.add("e", Bucket::Next, None);
        let done = s.complete(c).unwrap();
        assert_eq!(done.task.id, c);
        assert_eq!(done.pulled, None, "only the current task pulls from Next");
        assert_eq!(ids(&s, Bucket::Side), vec![b, d]);
        assert_eq!(s.current().unwrap().id, a);
        assert_eq!(ids(&s, Bucket::Next), vec![e]);
        assert!(s.undo_complete(done));
        assert_eq!(ids(&s, Bucket::Side), vec![b, c, d]);
        assert_eq!(s.current().unwrap().id, a);
        assert!(
            s.get(c).unwrap().started_at.is_none(),
            "only the current task is timed"
        );
        assert!(s.complete(99).is_none());
    }

    #[test]
    fn undo_complete_refuses_a_task_that_already_exists() {
        let mut s = Store::new();
        s.add("a", Bucket::Now, None);
        let done = s.complete_current().unwrap();
        assert!(s.undo_complete(done.clone()));
        assert!(!s.undo_complete(done));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn move_and_shift() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Next, None);
        let b = s.add("b", Bucket::Next, None);
        let c = s.add("c", Bucket::Next, None);
        assert!(s.shift(c, -1));
        assert_eq!(ids(&s, Bucket::Next), vec![a, c, b]);
        assert!(!s.shift(a, -1));
        assert!(s.shift(a, 5));
        assert_eq!(ids(&s, Bucket::Next), vec![c, b, a]);
        s.move_to(b, Bucket::Side, None);
        assert_eq!(ids(&s, Bucket::Side), vec![b]);
        assert_eq!(ids(&s, Bucket::Next), vec![c, a]);
        s.move_to(b, Bucket::Next, Some(1));
        assert_eq!(ids(&s, Bucket::Next), vec![c, b, a]);
        s.move_to(b, Bucket::Next, Some(99));
        assert_eq!(ids(&s, Bucket::Next), vec![c, a, b]);
    }

    #[test]
    fn quick_add_syntax() {
        let q = QuickAdd::parse("!fix the build #w").unwrap();
        assert_eq!(q.title, "fix the build");
        assert_eq!(q.bucket, Some(Bucket::Now));
        assert_eq!(q.tag, Some(Tag::Work));
        let q = QuickAdd::parse("call mum @later #personal").unwrap();
        assert_eq!(q.title, "call mum");
        assert_eq!(q.bucket, Some(Bucket::Later));
        assert_eq!(q.tag, Some(Tag::Personal));
        assert!(QuickAdd::parse("  #w ").is_none());
        let q = QuickAdd::parse("issue #123").unwrap();
        assert_eq!(q.title, "issue #123");
        // The quick-add placeholder advertises "!now" as a word: honour it
        // wherever it appears, and never leave it in the title.
        for input in [
            "!now fix the build",
            "fix the build !now",
            "! fix the build",
        ] {
            let q = QuickAdd::parse(input).unwrap();
            assert_eq!(q.title, "fix the build", "{input}");
            assert_eq!(q.bucket, Some(Bucket::Now), "{input}");
        }
        assert!(QuickAdd::parse("!now").is_none(), "nothing left to add");
        let q = QuickAdd::parse("!important thing").unwrap();
        assert_eq!(q.title, "important thing", "a bare ! still means Now");
        let mut s = Store::new();
        let id = s.quick_add("!do it", Bucket::Next).unwrap();
        assert_eq!(s.current().unwrap().id, id);
    }

    #[test]
    fn tags_and_rename() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Next, None);
        s.cycle_tag(a);
        assert_eq!(s.get(a).unwrap().tag, Some(Tag::Work));
        s.cycle_tag(a);
        assert_eq!(s.get(a).unwrap().tag, Some(Tag::Personal));
        s.cycle_tag(a);
        assert_eq!(s.get(a).unwrap().tag, None);
        assert!(s.rename(a, "  new "));
        assert_eq!(s.get(a).unwrap().title, "new");
        assert!(!s.rename(a, "  "));
    }

    #[test]
    fn task_titles_are_bounded_at_every_model_ingress() {
        let oversized = "🦀".repeat(MAX_TITLE_CHARS + 20);
        let mut store = Store::new();
        let id = store.add(&oversized, Bucket::Next, None);
        assert_eq!(
            store.get(id).unwrap().title.chars().count(),
            MAX_TITLE_CHARS
        );

        let replacement = "λ".repeat(MAX_TITLE_CHARS + 1);
        assert!(store.rename(id, &replacement));
        assert_eq!(
            store.get(id).unwrap().title.chars().count(),
            MAX_TITLE_CHARS
        );
        assert!(store.get(id).unwrap().title.chars().all(|c| c == 'λ'));

        let json = serde_json::json!({
            "next_id": 2,
            "tasks": [{
                "id": 1,
                "title": oversized,
                "bucket": "next",
                "created_at": 1
            }]
        });
        let loaded: Store = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.tasks[0].title.chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn quick_add_refuses_input_too_large_to_parse_safely() {
        let oversized = "x".repeat(MAX_QUICK_ADD_BYTES + 1);
        assert!(QuickAdd::parse(&oversized).is_none());
    }

    #[test]
    fn pause_freezes_the_clock_and_resume_keeps_it() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, None);
        // Pretend the task has been running for a minute.
        let now = unix_now();
        s.tasks[0].started_at = Some(now - 60);

        assert!(s.toggle_pause());
        assert!(s.get(a).unwrap().is_paused());
        let task = s.get(a).unwrap();
        assert_eq!(
            task.elapsed_secs(now + 30),
            task.elapsed_secs(now + 3000),
            "the clock is frozen while paused"
        );
        assert!((60..=61).contains(&task.elapsed_secs(now).unwrap()));

        // Resuming keeps the 60s already on the clock rather than restarting.
        assert!(s.toggle_pause());
        assert!(!s.get(a).unwrap().is_paused());
        assert!((60..=61).contains(&s.get(a).unwrap().elapsed_secs(unix_now()).unwrap()));
    }

    #[test]
    fn pause_needs_a_current_task_and_never_outlives_it() {
        let mut s = Store::new();
        assert!(!s.toggle_pause(), "nothing in Now");
        let a = s.add("a", Bucket::Now, None);
        let b = s.add("b", Bucket::Next, None);
        assert!(s.toggle_pause());
        assert!(s.get(a).unwrap().is_paused());

        // Losing the current slot clears the pause with the timer.
        s.promote(b);
        assert!(!s.get(a).unwrap().is_paused());
        assert!(s.get(a).unwrap().started_at.is_none());
        assert!(!s.get(b).unwrap().is_paused());
    }

    /// `paused_at` is the contract with the shell extension, which freezes its
    /// own clock on it. Pin the key name and the round-trip.
    #[test]
    fn a_paused_task_survives_the_snapshot_and_the_file() {
        let mut s = Store::new();
        s.add("a", Bucket::Now, Some(Tag::Work));
        s.tasks[0].started_at = Some(1000);
        assert!(s.toggle_pause());
        let paused_at = s.current().unwrap().paused_at;
        assert!(paused_at.is_some());

        let v: serde_json::Value = serde_json::from_str(&s.snapshot_json()).unwrap();
        assert_eq!(v["current"]["paused_at"], serde_json::json!(paused_at));
        assert_eq!(v["now"][0]["paused_at"], serde_json::json!(paused_at));

        let back: Store = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.tasks, s.tasks);
        assert!(back.current().unwrap().is_paused());
    }

    #[test]
    fn tasks_without_the_pause_field_still_load() {
        let json = r#"{"next_id":2,"tasks":[{"id":1,"title":"a","bucket":"now",
            "created_at":0,"started_at":10}]}"#;
        let s: Store = serde_json::from_str(json).unwrap();
        assert_eq!(s.current().unwrap().paused_at, None);
        assert_eq!(s.current().unwrap().elapsed_secs(70), Some(60));
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut s = Store::new();
        s.add("a", Bucket::Now, Some(Tag::Work));
        s.add("b", Bucket::Side, None);
        let json = s.snapshot_json();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["current"]["title"], "a");
        assert_eq!(v["current"]["tag"], "work");
        assert_eq!(v["side"][0]["title"], "b");
        let ser = serde_json::to_string(&s).unwrap();
        let back: Store = serde_json::from_str(&ser).unwrap();
        assert_eq!(back.tasks, s.tasks);
    }
}
