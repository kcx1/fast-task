//! The share's activity log: who changed what while a LAN share was running.
//!
//! Entries are written from the same places that record undo history
//! (`append_history`, undo, redo) plus notes, so every write path is covered
//! without each caller having to remember. The author comes from
//! [`with_actor`]: the share server wraps a browser's writes in it; anything
//! else (the desktop UI) is logged as "You". Nothing is logged while not
//! sharing — undo history already covers editing alone.

use std::cell::RefCell;

use polodb_core::CollectionT;
use polodb_core::bson::{self, doc, oid::ObjectId};

use crate::database::database::Db;
use crate::database::history::{Event, SaveableItem};
pub use crate::database::models::ActivityEntry;
use crate::database::models::{ActivityAction, Task, TaskStatus};

pub(crate) const ACTIVITY_COLLECTION: &str = "share_activity";
/// Oldest entries are pruned past this many.
pub const ACTIVITY_LIMIT: usize = 500;

thread_local! {
    static ACTOR: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Run `f` with its writes attributed to `who` in the activity log. Writes are
/// synchronous, so a thread-local reaches every write `f` makes.
pub fn with_actor<R>(who: &str, f: impl FnOnce() -> R) -> R {
    struct Reset(Option<String>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTOR.with(|a| *a.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(ACTOR.with(|a| a.borrow_mut().replace(who.to_string())));
    f()
}

/// A note's first line, cut to fit the log.
pub(crate) fn excerpt(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    const MAX: usize = 80;
    if line.chars().count() > MAX {
        format!("{}…", line.chars().take(MAX).collect::<String>())
    } else {
        line.to_string()
    }
}

/// Logged when a note is added / deleted.
pub(crate) fn note_change(action: ActivityAction, task_title: &str, note: Option<&str>) -> Change {
    let change = Change::new(action, task_title);
    match note {
        Some(text) => change.detail(excerpt(text)),
        None => change,
    }
}

pub(crate) fn quoted(title: &str) -> String {
    const MAX: usize = 60;
    if title.chars().count() > MAX {
        format!("“{}…”", title.chars().take(MAX).collect::<String>())
    } else {
        format!("“{title}”")
    }
}

/// One logged change: what kind, to what, and any extra detail.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub action: ActivityAction,
    pub subject: String,
    pub detail: Option<String>,
}

impl Change {
    fn new(action: ActivityAction, subject: &str) -> Self {
        Self {
            action,
            subject: subject.to_string(),
            detail: None,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// The change as one sentence (the log entry's `what`).
    pub fn sentence(&self) -> String {
        let mut s = format!("{} {}", self.action.verb(), quoted(&self.subject));
        if let Some(detail) = &self.detail {
            s.push_str(&format!(" ({detail})"));
        }
        s
    }
}

fn status_action(status: &TaskStatus) -> ActivityAction {
    match status {
        TaskStatus::NotStarted => ActivityAction::Reopened,
        TaskStatus::InProgress => ActivityAction::Started,
        TaskStatus::OnHold => ActivityAction::OnHold,
        TaskStatus::Completed => ActivityAction::Completed,
    }
}

/// What changed between two versions of a task. The most telling change is
/// the action; the rest go in the detail. `None` when nothing a person would
/// care about changed.
fn describe_task_update(before: &Task, after: &Task) -> Option<Change> {
    let mut actions = Vec::new();
    let mut details = Vec::new();
    if before.status != after.status {
        actions.push(status_action(&after.status));
    }
    if before.title != after.title {
        actions.push(ActivityAction::Renamed);
        details.push(format!("was {}", quoted(&before.title)));
    }
    if before.priority != after.priority {
        actions.push(ActivityAction::Priority);
        details.push(format!("{} priority", after.priority));
    }
    if before.details != after.details {
        details.push("details".to_string());
    }
    let fields = [
        (before.due != after.due, "due date"),
        (before.wait_until != after.wait_until, "wait date"),
        (before.tags != after.tags, "tags"),
        (before.recurrence != after.recurrence, "repeat"),
        (
            before.code != after.code || before.language != after.language,
            "format",
        ),
        (before.project_id != after.project_id, "project"),
    ];
    details.extend(
        fields
            .iter()
            .filter(|(c, _)| *c)
            .map(|(_, f)| f.to_string()),
    );

    let action = match actions.first() {
        Some(action) => *action,
        None if !details.is_empty() => ActivityAction::Edited,
        None if before.order != after.order => ActivityAction::Moved,
        None => return None,
    };
    let change = Change::new(action, &after.title);
    Some(if details.is_empty() {
        change
    } else {
        change.detail(details.join(" · "))
    })
}

/// The log line for a history event.
pub fn describe(event: &Event) -> Option<Change> {
    use ActivityAction::*;
    Some(match event {
        Event::Create(SaveableItem::Task(t)) => Change::new(Added, &t.title),
        Event::Create(SaveableItem::Project(p)) => Change::new(Added, &p.name).detail("project"),
        Event::Delete(SaveableItem::Task(t)) => Change::new(Deleted, &t.title),
        Event::Delete(SaveableItem::Project(p)) => Change::new(Deleted, &p.name).detail("project"),
        Event::Update {
            before: SaveableItem::Task(b),
            after: SaveableItem::Task(a),
        } => describe_task_update(b, a)?,
        Event::Update {
            before: SaveableItem::Project(b),
            after: SaveableItem::Project(a),
        } => {
            if b.name == a.name {
                return None;
            }
            Change::new(Renamed, &a.name).detail(format!("project, was {}", quoted(&b.name)))
        }
        Event::Update { .. } => return None,
    })
}

/// An undo / redo of `event`: same subject, the original change as detail.
pub(crate) fn describe_reversal(event: &Event, action: ActivityAction) -> Option<Change> {
    let original = describe(event)?;
    let mut detail = capitalized(original.action.verb());
    if let Some(d) = original.detail {
        detail.push_str(&format!(" · {d}"));
    }
    Some(Change::new(action, &original.subject).detail(detail))
}

fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

impl Db {
    /// Start or stop recording activity (on while a share runs).
    pub fn set_activity_logging(&self, on: bool) {
        self.activity_on
            .store(on, std::sync::atomic::Ordering::SeqCst);
    }

    /// Record `what` for the current actor, if logging is on. Never fails the
    /// write it describes: the log is best-effort.
    pub(crate) fn log_activity(&self, change: impl FnOnce() -> Option<Change>) {
        if !self.activity_on.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let Some(change) = change() else {
            return;
        };
        let entry = ActivityEntry {
            id: ObjectId::new(),
            at: bson::DateTime::now(),
            who: ACTOR.with(|a| a.borrow().clone()),
            what: change.sentence(),
            action: Some(change.action),
            subject: Some(change.subject),
            detail: change.detail,
        };
        let col = self
            .instance
            .collection::<ActivityEntry>(ACTIVITY_COLLECTION);
        if col.insert_one(entry).is_err() {
            return;
        }
        // Prune in batches, so most writes skip the extra work.
        if col.count_documents().unwrap_or(0) as usize > ACTIVITY_LIMIT + 50 {
            let _ = self.prune_activity();
        }
    }

    fn prune_activity(&self) -> anyhow::Result<()> {
        let col = self
            .instance
            .collection::<ActivityEntry>(ACTIVITY_COLLECTION);
        let stale: Vec<ObjectId> = col
            .find(doc! {})
            .sort(doc! { "_id": -1 })
            .skip(ACTIVITY_LIMIT as u64)
            .run()?
            .filter_map(Result::ok)
            .map(|e| e.id)
            .collect();
        for id in stale {
            col.delete_one(doc! { "_id": id })?;
        }
        Ok(())
    }

    /// Newest first, at most `limit`.
    pub fn activity(&self, limit: usize) -> anyhow::Result<Vec<ActivityEntry>> {
        Ok(self
            .instance
            .collection::<ActivityEntry>(ACTIVITY_COLLECTION)
            .find(doc! {})
            .sort(doc! { "_id": -1 })
            .limit(limit as u64)
            .run()?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn clear_activity(&self) -> anyhow::Result<()> {
        self.write(|| {
            self.instance
                .collection::<ActivityEntry>(ACTIVITY_COLLECTION)
                .delete_many(doc! {})?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::TaskManagement;

    fn test_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("t.db")).unwrap();
        (dir, db)
    }

    /// (who, action, subject, detail), oldest first.
    type Row = (Option<String>, ActivityAction, String, Option<String>);

    fn rows(db: &Db) -> Vec<Row> {
        let mut entries = db.activity(100).unwrap();
        entries.reverse();
        entries
            .into_iter()
            .map(|e| (e.who, e.action.unwrap(), e.subject.unwrap(), e.detail))
            .collect()
    }

    #[test]
    fn logs_only_while_sharing_with_the_right_author() {
        use ActivityAction::*;
        let (_dir, db) = test_db();
        let id = db
            .create_task(Task {
                title: "Fix tap".into(),
                ..Default::default()
            })
            .unwrap();
        assert!(rows(&db).is_empty(), "not sharing: nothing logged");

        db.set_activity_logging(true);
        with_actor("Sam", || {
            db.set_status(id, &TaskStatus::Completed).unwrap();
        });
        db.modify_task(id, &mut |t| {
            t.title = "Fix kitchen tap".into();
            t.priority = crate::database::models::Priority::Urgent;
        })
        .unwrap();
        db.undo().unwrap();
        db.delete_task(id).unwrap();

        let was = Some("was “Fix tap” · Urgent priority".to_string());
        assert_eq!(
            rows(&db),
            vec![
                (Some("Sam".into()), Completed, "Fix tap".into(), None),
                (None, Renamed, "Fix kitchen tap".into(), was),
                (
                    None,
                    Undid,
                    "Fix kitchen tap".into(),
                    Some("Renamed · was “Fix tap” · Urgent priority".into())
                ),
                (None, Deleted, "Fix tap".into(), None),
            ]
        );
        let entries = db.activity(1).unwrap();
        assert_eq!(entries[0].what, "deleted “Fix tap”");
    }

    #[test]
    fn notes_and_field_edits_are_described() {
        use ActivityAction::*;
        let (_dir, db) = test_db();
        let id = db
            .create_task(Task {
                title: "t".into(),
                ..Default::default()
            })
            .unwrap();
        db.set_activity_logging(true);
        db.modify_task(id, &mut |t| t.tags = Some(vec!["a".into()]))
            .unwrap();
        let note = db
            .add_annotation(crate::database::models::Annotation {
                id: ObjectId::new(),
                task_id: id,
                content: "first line\nsecond".into(),
                created_at: bson::DateTime::now(),
                author: None,
            })
            .unwrap();
        db.delete_annotation(note).unwrap();
        assert_eq!(
            rows(&db),
            vec![
                (None, Edited, "t".into(), Some("tags".into())),
                (None, Noted, "t".into(), Some("first line".into())),
                (None, NoteDeleted, "t".into(), Some("first line".into())),
            ]
        );
    }

    #[test]
    fn actor_is_reset_after_the_scope() {
        let (_dir, db) = test_db();
        db.set_activity_logging(true);
        with_actor("Sam", || {});
        db.create_task(Task {
            title: "t".into(),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(rows(&db)[0].0, None);
    }

    #[test]
    fn no_op_updates_are_not_logged() {
        let (_dir, db) = test_db();
        let id = db.create_task(Task::default()).unwrap();
        db.set_activity_logging(true);
        db.modify_task(id, &mut |_| {}).unwrap();
        assert!(rows(&db).is_empty());
    }

    #[test]
    fn prunes_to_the_limit() {
        let (_dir, db) = test_db();
        db.set_activity_logging(true);
        for i in 0..(ACTIVITY_LIMIT + 60) {
            db.log_activity(|| Some(Change::new(ActivityAction::Added, &format!("entry {i}"))));
        }
        let entries = db.activity(usize::MAX).unwrap();
        assert!(entries.len() <= ACTIVITY_LIMIT + 50);
        assert_eq!(
            entries[0].subject.as_deref(),
            Some(format!("entry {}", ACTIVITY_LIMIT + 59).as_str())
        );
    }
}
