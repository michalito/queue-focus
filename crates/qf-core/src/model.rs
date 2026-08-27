use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

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
    pub title: String,
    pub bucket: Bucket,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<Tag>,
    pub created_at: u64,
    /// Unix seconds since this task became the current (head of Now) task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
}

impl Task {
    pub fn elapsed_secs(&self, now: u64) -> Option<u64> {
        self.started_at.map(|s| now.saturating_sub(s))
    }
}

/// Result of parsing quick-add syntax:
/// `!title` -> Now, `title #work` / `#w` / `#p`, `@later` / `@side` / `@next` / `@now`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickAdd {
    pub title: String,
    pub bucket: Option<Bucket>,
    pub tag: Option<Tag>,
}

impl QuickAdd {
    pub fn parse(input: &str) -> Option<QuickAdd> {
        let mut bucket = None;
        let mut tag = None;
        let mut words: Vec<&str> = Vec::new();
        for w in input.split_whitespace() {
            if let Some(t) = w.strip_prefix('#').and_then(Tag::parse) {
                tag = Some(t);
            } else if let Some(b) = w.strip_prefix('@').and_then(Bucket::parse) {
                bucket = Some(b);
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

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ordered task store. `tasks` order is the display order within each bucket.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default = "one")]
    pub next_id: u64,
    #[serde(default)]
    pub tasks: Vec<Task>,
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

    /// The focused task: head of Now.
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

    pub fn add(&mut self, title: &str, bucket: Bucket, tag: Option<Tag>, front: bool) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let task = Task {
            id,
            title: title.trim().to_string(),
            bucket,
            tag,
            created_at: unix_now(),
            started_at: None,
        };
        if front {
            let pos = self.first_pos(bucket).unwrap_or(self.tasks.len());
            self.tasks.insert(pos, task);
        } else {
            self.tasks.push(task);
        }
        self.normalize();
        id
    }

    pub fn quick_add(&mut self, input: &str, default_bucket: Bucket) -> Option<u64> {
        let q = QuickAdd::parse(input)?;
        let bucket = q.bucket.unwrap_or(default_bucket);
        let front = bucket == Bucket::Now;
        Some(self.add(&q.title, bucket, q.tag, front))
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

    /// Delete the current task; if Now becomes empty, pull the head of Next.
    pub fn complete_current(&mut self) -> Option<Task> {
        let id = self.current()?.id;
        let idx = self.tasks.iter().position(|t| t.id == id)?;
        let done = self.tasks.remove(idx);
        if self.current().is_none() {
            let next = self.in_bucket(Bucket::Next).next().map(|t| t.id);
            if let Some(next) = next {
                self.move_to(next, Bucket::Now, Some(0));
            }
        }
        self.normalize();
        Some(done)
    }

    /// Make a task the current one (head of Now).
    pub fn promote(&mut self, id: u64) -> bool {
        self.move_to(id, Bucket::Now, Some(0))
    }

    /// Move a task into `bucket` at `index` (None = end).
    pub fn move_to(&mut self, id: u64, bucket: Bucket, index: Option<usize>) -> bool {
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
        self.normalize();
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

    pub fn rename(&mut self, id: u64, title: &str) -> bool {
        let title = title.trim();
        if title.is_empty() {
            return false;
        }
        match self.tasks.iter_mut().find(|t| t.id == id) {
            Some(t) => {
                t.title = title.to_string();
                true
            }
            None => false,
        }
    }

    /// Only the current task carries a running timer.
    fn normalize(&mut self) {
        let cur = self.current().map(|t| t.id);
        let now = unix_now();
        for t in &mut self.tasks {
            if Some(t.id) == cur {
                if t.started_at.is_none() {
                    t.started_at = Some(now);
                }
            } else {
                t.started_at = None;
            }
        }
    }

    fn first_pos(&self, bucket: Bucket) -> Option<usize> {
        self.tasks.iter().position(|t| t.bucket == bucket)
    }

    /// Compact JSON snapshot for the shell extension / CLI.
    pub fn snapshot_json(&self) -> String {
        let task = |t: &Task| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "tag": t.tag.map(Tag::as_str),
                "started_at": t.started_at,
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
        let a = s.add("a", Bucket::Next, None, false);
        let b = s.add("b", Bucket::Next, None, false);
        let c = s.add("c", Bucket::Next, None, true);
        assert_eq!(ids(&s, Bucket::Next), vec![c, a, b]);
        assert!(s.current().is_none());
    }

    #[test]
    fn promote_and_timer() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Next, None, false);
        let b = s.add("b", Bucket::Later, None, false);
        assert!(s.promote(b));
        assert_eq!(s.current().unwrap().id, b);
        assert!(s.get(b).unwrap().started_at.is_some());
        assert!(s.get(a).unwrap().started_at.is_none());
        s.promote(a);
        assert_eq!(ids(&s, Bucket::Now), vec![a, b]);
        assert!(s.get(b).unwrap().started_at.is_none(), "only head is timed");
    }

    #[test]
    fn complete_pulls_from_next() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, None, false);
        let b = s.add("b", Bucket::Next, None, false);
        let c = s.add("c", Bucket::Next, None, false);
        let done = s.complete_current().unwrap();
        assert_eq!(done.id, a);
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
    fn complete_does_not_pull_when_now_has_more() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Now, None, false);
        let b = s.add("b", Bucket::Now, None, false);
        let c = s.add("c", Bucket::Next, None, false);
        s.complete_current();
        assert_eq!(s.current().unwrap().id, b);
        assert_eq!(ids(&s, Bucket::Next), vec![c]);
        let _ = a;
    }

    #[test]
    fn move_and_shift() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Next, None, false);
        let b = s.add("b", Bucket::Next, None, false);
        let c = s.add("c", Bucket::Next, None, false);
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
        let mut s = Store::new();
        let id = s.quick_add("!do it", Bucket::Next).unwrap();
        assert_eq!(s.current().unwrap().id, id);
    }

    #[test]
    fn tags_and_rename() {
        let mut s = Store::new();
        let a = s.add("a", Bucket::Next, None, false);
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
    fn snapshot_roundtrip() {
        let mut s = Store::new();
        s.add("a", Bucket::Now, Some(Tag::Work), false);
        s.add("b", Bucket::Side, None, false);
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
