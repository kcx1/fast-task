use crate::database::history::SaveableItem;
use crate::database::history::{Event, HistoryRecord};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Context;
use dirs::data_dir;
use polodb_core::CollectionT;
use polodb_core::bson::{self, oid::ObjectId};
use polodb_core::bson::{doc, to_bson};
use serde::{Deserialize, Serialize};

use crate::database::Task;
use crate::database::models::{Annotation, Tag};
use crate::database::{Database, Project, ProjectManagement, TagManagement};
use crate::database::{ProjectEntry, TaskManagement};

pub(crate) const TASK_COLLECTION: &str = "tasks";
const PROJECT_COLLECTION: &str = "projects";
const APP_STATE: &str = "app_state";
pub(crate) const HISTORY_COLLECTION: &str = "history";
const TAGS_COLLECTION: &str = "tags";
const ANNOTATIONS_COLLECTION: &str = "annotations";

pub static DATABASE: LazyLock<PathBuf> = LazyLock::new(|| {
    data_dir()
        .expect("Cannot determine system data directory — HOME may not be set")
        .join("todo.db")
});

/// Persisted app state (last selected project filter). Distinct from the in-memory UI `PersistedState` in `app.rs`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PersistedState {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub project_filter: ProjectEntry,
}

#[derive(Clone)]
pub struct Db {
    pub(crate) instance: polodb_core::Database,
    redo_stack: Arc<Mutex<Vec<Event>>>,
}

const RECENT_ID: &str = "000000000000000000000001";

impl Db {
    pub fn open_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let db = Self {
            redo_stack: Arc::new(Mutex::new(Vec::new())),
            instance: polodb_core::Database::open_path(path.as_ref())
                .context("Failed to open DB")?,
        };
        crate::database::migrations::run(&db.instance)?;
        Ok(db)
    }
}

impl Database for Db {
    fn open() -> anyhow::Result<Self> {
        Self::open_path(DATABASE.as_path())
    }

    fn close() -> anyhow::Result<()> {
        // PoloDB flushes on drop; no explicit close needed.
        Ok(())
    }
}

impl ProjectManagement for Db {
    fn all_projects(&self) -> anyhow::Result<Vec<ProjectEntry>> {
        let mut result = vec![];
        self.instance
            .collection::<Project>(PROJECT_COLLECTION)
            .find(doc! {})
            .run()
            .context("Error finding all projects")?
            .flatten()
            .for_each(|doc| result.push(ProjectEntry::Project(doc)));

        Ok(result)
    }

    fn one_project(&self, project_id: ObjectId) -> anyhow::Result<Option<ProjectEntry>> {
        Ok(self
            .instance
            .collection::<Project>(PROJECT_COLLECTION)
            .find_one(doc! {"_id": project_id})
            .context("Error accessing the database")?
            .map(ProjectEntry::Project))
    }

    fn create_project(&self, project: Project) -> anyhow::Result<ObjectId> {
        let id = self
            .instance
            .collection(PROJECT_COLLECTION)
            .insert_one(project.clone())?
            .inserted_id
            .as_object_id()
            .unwrap(); // Should be safe since the insert raises the error

        // Append History
        let event = Event::Create(SaveableItem::Project(project));

        self.append_history(event)?;
        Ok(id)
    }

    fn delete_project(&self, project_id: ObjectId) -> anyhow::Result<()> {
        // Record a Delete event for every task so undo can restore them individually.
        let tasks: Vec<Task> = self
            .instance
            .collection::<Task>(TASK_COLLECTION)
            .find(doc! { "project_id": project_id })
            .run()?
            .collect::<Result<_, _>>()?;

        for task in tasks {
            self.append_history(Event::Delete(SaveableItem::Task(task.clone())))?;
            self.instance
                .collection::<Task>(TASK_COLLECTION)
                .delete_one(doc! { "_id": task.id })?;
        }

        // Record the project delete last so undo restores it before re-linking tasks.
        let project = self
            .instance
            .collection::<Project>(PROJECT_COLLECTION)
            .find_one(doc! { "_id": project_id })?
            .ok_or_else(|| anyhow::anyhow!("Project not found"))?;

        self.append_history(Event::Delete(SaveableItem::Project(project)))?;

        self.instance
            .collection::<Project>(PROJECT_COLLECTION)
            .delete_one(doc! {"_id": project_id})?;

        Ok(())
    }

    fn update_project(&self, project: Project) -> anyhow::Result<ObjectId> {
        let before = self
            .instance
            .collection::<Project>(PROJECT_COLLECTION)
            .find_one(doc! { "_id": project.id })
            .context("Failed to read project from database")?;

        self.instance
            .collection::<Project>(PROJECT_COLLECTION)
            .update_one(
                doc! { "_id": project.id },
                doc! { "$set": project_set_doc(&project) },
            )?;

        if let Some(before) = before {
            self.append_history(Event::Update {
                before: SaveableItem::Project(before),
                after: SaveableItem::Project(project.clone()),
            })?;
        }

        Ok(project.id)
    }
}

impl TaskManagement for Db {
    fn one_task(&self, task_id: bson::oid::ObjectId) -> anyhow::Result<Option<Task>> {
        self.instance
            .collection::<Task>(TASK_COLLECTION)
            .find_one(doc! {"_id": task_id})
            .context("Failed to read task from database")
    }

    fn get_tasks(&self, lookup: ProjectEntry) -> anyhow::Result<Vec<Task>> {
        let filter = lookup.task_lookup();
        let tasks = self
            .instance
            .collection::<Task>(TASK_COLLECTION)
            .find(filter)
            .sort(doc! { "order": 1 })
            .run()?;

        tasks
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to deserialize task row")
    }

    fn create_task(&self, task: Task) -> anyhow::Result<ObjectId> {
        let id = self
            .instance
            .collection::<Task>(TASK_COLLECTION)
            .insert_one(&task)?
            .inserted_id
            .as_object_id()
            .unwrap();

        if let Some(tags) = &task.tags {
            self.upsert_tags(tags)?;
        }

        let event = Event::Create(SaveableItem::Task(task.clone()));
        self.append_history(event)?;

        Ok(id)
    }

    fn delete_task(&self, task_id: ObjectId) -> anyhow::Result<Task> {
        let task = self.one_task(task_id)?;
        let res = self
            .instance
            .collection::<Task>(TASK_COLLECTION)
            .delete_one(doc! {"_id": task_id})?;

        if let Some(deleted_task) = task
            && res.deleted_count > 0
        {
            let event = Event::Delete(SaveableItem::Task(deleted_task.clone()));

            self.append_history(event)?;
            return Ok(deleted_task);
        }
        Err(anyhow::anyhow!("Task not found"))
    }

    fn update_task(&self, task: Task) -> anyhow::Result<ObjectId> {
        let before = self.one_task(task.id)?;
        self.instance
            .collection::<Task>(TASK_COLLECTION)
            .update_one(
                doc! {"_id": &task.id},
                doc! { "$set": task_set_doc(&task)? },
            )?;

        if let Some(tags) = &task.tags {
            self.upsert_tags(tags)?;
        }

        if let Some(before) = before {
            let event = Event::Update {
                before: SaveableItem::Task(before),
                after: SaveableItem::Task(task.clone()),
            };

            self.append_history(event)?;
        }

        Ok(task.id)
    }
}

impl TagManagement for Db {
    fn all_tags(&self) -> anyhow::Result<Vec<String>> {
        Ok(self
            .instance
            .collection::<Tag>(TAGS_COLLECTION)
            .find(doc! {})
            .run()?
            .flatten()
            .map(|t| t.content)
            .collect())
    }

    fn upsert_tags(&self, tags: &[String]) -> anyhow::Result<()> {
        let col = self.instance.collection::<Tag>(TAGS_COLLECTION);
        for tag in tags {
            let normalized = tag.trim().to_lowercase();
            if normalized.is_empty() {
                continue;
            }
            if col.find_one(doc! { "content": &normalized })?.is_none() {
                col.insert_one(Tag {
                    content: normalized,
                })?;
            }
        }
        Ok(())
    }
}

/// Returns the canonical `$set` document for a task update.
/// Used by `update_task`, `undo`, and `redo` so all three stay in sync.
fn task_set_doc(task: &Task) -> anyhow::Result<bson::Document> {
    Ok(doc! {
        "single_line": &task.title,
        "details":     &task.details,
        "code":        &task.code,
        "status":      to_bson(&task.status)?,
        "due":         &task.due,
        "wait_until":  &task.wait_until,
        "order":       task.order as i64,
        "priority":    &task.priority,
        "tags":        &task.tags,
        "recurrence":  to_bson(&task.recurrence)?,
    })
}

/// Returns the canonical `$set` document for a project update.
fn project_set_doc(project: &Project) -> bson::Document {
    doc! {
        "name": &project.name,
        "tags": &project.tags,
    }
}

impl Db {
    pub fn save_current_project(&self, project: ProjectEntry) -> anyhow::Result<()> {
        let id = ObjectId::from_str(RECENT_ID).expect("Invalid Database ID");
        let col = self.instance.collection::<PersistedState>(APP_STATE);
        // Delete before insert so we always have exactly one record (PoloDB lacks upsert)
        col.delete_one(doc! { "_id": &id }).ok();
        col.insert_one(PersistedState {
            id,
            project_filter: project,
        })?;
        Ok(())
    }

    pub fn get_recent_project(&self) -> Option<ProjectEntry> {
        let recent_app_state: Option<PersistedState> = self
            .instance
            .collection(APP_STATE)
            .find_one(doc! {"_id": ObjectId::from_str(RECENT_ID).ok()?})
            .ok()?;

        match recent_app_state {
            Some(app_state) => Some(app_state.project_filter),
            None => None,
        }
    }
    pub fn append_history(&self, event: Event) -> anyhow::Result<()> {
        let record = HistoryRecord {
            id: ObjectId::new(),
            timestamp: bson::DateTime::now(),
            event,
        };
        // Clear the redo stack before adding new history
        self.redo_stack.lock().unwrap().clear();

        self.instance
            .collection(HISTORY_COLLECTION)
            .insert_one(record)?;

        Ok(())
    }

    pub fn undo(&self) -> anyhow::Result<()> {
        let last = self
            .instance
            .collection::<HistoryRecord>(HISTORY_COLLECTION)
            .find(doc! {})
            .sort(doc! { "_id": -1 })
            .limit(1)
            .run()?
            .next()
            .transpose()?;

        if let Some(record) = last {
            let event = record.event.clone();
            match record.event {
                Event::Create(task) => match task {
                    SaveableItem::Task(task) => {
                        self.instance
                            .collection::<Task>(TASK_COLLECTION)
                            .delete_one(doc! { "_id": task.id })?;
                    }
                    SaveableItem::Project(project) => {
                        self.instance
                            .collection::<Project>(PROJECT_COLLECTION)
                            .delete_one(doc! { "_id": project.id })?;
                    }
                },

                Event::Update { before, .. } => match before {
                    SaveableItem::Task(task) => {
                        self.instance
                            .collection::<Task>(TASK_COLLECTION)
                            .update_one(
                                doc! { "_id": task.id },
                                doc! { "$set": task_set_doc(&task)? },
                            )?;
                    }
                    SaveableItem::Project(project) => {
                        self.instance
                            .collection::<Project>(PROJECT_COLLECTION)
                            .update_one(
                                doc! { "_id": project.id },
                                doc! { "$set": project_set_doc(&project) },
                            )?;
                    }
                },

                Event::Delete(task) => match task {
                    SaveableItem::Task(task) => {
                        self.instance
                            .collection::<Task>(TASK_COLLECTION)
                            .insert_one(task)?;
                    }
                    SaveableItem::Project(project) => {
                        self.instance
                            .collection::<Project>(PROJECT_COLLECTION)
                            .insert_one(project)?;
                    }
                },
            }
            self.redo_stack.lock().unwrap().push(event);
            // Remove the history entry (simple stack behavior)
            self.instance
                .collection::<HistoryRecord>(HISTORY_COLLECTION)
                .delete_one(doc! { "_id": record.id })?;
        }

        Ok(())
    }

    pub fn redo(&self) -> anyhow::Result<()> {
        let event = {
            let mut redo = self.redo_stack.lock().unwrap();
            redo.pop()
        };

        if let Some(event) = event {
            match &event {
                Event::Create(item) => match item {
                    SaveableItem::Task(task) => {
                        self.instance
                            .collection::<Task>(TASK_COLLECTION)
                            .insert_one(task.clone())?;
                    }
                    SaveableItem::Project(project) => {
                        self.instance
                            .collection::<Project>(PROJECT_COLLECTION)
                            .insert_one(project.clone())?;
                    }
                },

                Event::Update { after, .. } => match after {
                    SaveableItem::Task(task) => {
                        self.instance
                            .collection::<Task>(TASK_COLLECTION)
                            .update_one(
                                doc! { "_id": task.id },
                                doc! { "$set": task_set_doc(task)? },
                            )?;
                    }
                    SaveableItem::Project(project) => {
                        self.instance
                            .collection::<Project>(PROJECT_COLLECTION)
                            .update_one(
                                doc! { "_id": project.id },
                                doc! { "$set": project_set_doc(project) },
                            )?;
                    }
                },

                Event::Delete(item) => match item {
                    SaveableItem::Task(task) => {
                        self.instance
                            .collection::<Task>(TASK_COLLECTION)
                            .delete_one(doc! { "_id": task.id })?;
                    }
                    SaveableItem::Project(project) => {
                        self.instance
                            .collection::<Project>(PROJECT_COLLECTION)
                            .delete_one(doc! { "_id": project.id })?;
                    }
                },
            }

            // Push back to history without clearing the redo stack so chained redos work
            let record = HistoryRecord {
                id: ObjectId::new(),
                timestamp: bson::DateTime::now(),
                event,
            };
            self.instance
                .collection(HISTORY_COLLECTION)
                .insert_one(record)?;
        }

        Ok(())
    }
    pub fn get_annotations(&self, task_id: ObjectId) -> anyhow::Result<Vec<Annotation>> {
        self.instance
            .collection::<Annotation>(ANNOTATIONS_COLLECTION)
            .find(doc! { "task_id": task_id })
            .sort(doc! { "_id": 1 })
            .run()?
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to deserialize annotations")
    }

    pub fn add_annotation(&self, annotation: Annotation) -> anyhow::Result<ObjectId> {
        Ok(self
            .instance
            .collection::<Annotation>(ANNOTATIONS_COLLECTION)
            .insert_one(annotation)?
            .inserted_id
            .as_object_id()
            .unwrap())
    }

    pub fn delete_annotation(&self, annotation_id: ObjectId) -> anyhow::Result<()> {
        self.instance
            .collection::<Annotation>(ANNOTATIONS_COLLECTION)
            .delete_one(doc! { "_id": annotation_id })?;
        Ok(())
    }

    pub fn load_history(&self) -> anyhow::Result<Vec<HistoryRecord>> {
        let records = self
            .instance
            .collection::<HistoryRecord>(HISTORY_COLLECTION)
            .find(doc! {})
            .sort(doc! { "_id": 1 })
            .run()?;

        records
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to deserialize history record")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::models::Priority;
    use polodb_core::bson::Document;
    use tempfile::TempDir;

    // TempDir must be first so it outlives Db (Rust drops in reverse declaration order)
    fn test_db() -> (TempDir, Db) {
        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        (dir, db)
    }

    fn make_project(name: &str) -> Project {
        Project::new(name, None)
    }

    fn make_task(single_line: &str, project_id: Option<ObjectId>) -> Task {
        Task {
            title: single_line.to_string(),
            project_id,
            order: 1000,
            ..Default::default()
        }
    }

    // --- Migration tests ---

    #[test]
    fn migrations_run_on_open() {
        use crate::database::migrations::CURRENT_SCHEMA_VERSION;
        let (_dir, db) = test_db();
        let version = crate::database::migrations::run(&db.instance)
            .map(|_| CURRENT_SCHEMA_VERSION)
            .unwrap_or(0);
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn migration_001_renames_priority_typo() {
        let (_dir, db) = test_db();
        db.instance
            .collection::<Document>(TASK_COLLECTION)
            .insert_one(doc! {
                "_id": ObjectId::new(),
                "project_id": bson::Bson::Null,
                "single_line": "legacy task",
                "details": "",
                "code": false,
                "status": "NotStarted",
                "due": bson::Bson::Null,
                "priorty": "Normal",
                "tags": bson::Bson::Null,
                "modify_date": bson::DateTime::now(),
                "order": 1000i64,
            })
            .unwrap();

        crate::database::migrations::migration_001_fix_priority_typo(&db.instance).unwrap();

        let tasks = db.get_tasks(ProjectEntry::All).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].priority, Priority::Normal);
    }

    #[test]
    fn migration_002_removes_duplicate_priority_field() {
        let (_dir, db) = test_db();
        db.instance
            .collection::<Document>(TASK_COLLECTION)
            .insert_one(doc! {
                "_id": ObjectId::new(),
                "project_id": bson::Bson::Null,
                "single_line": "updated task",
                "details": "",
                "code": false,
                "status": "NotStarted",
                "due": bson::Bson::Null,
                "priorty": "Normal",
                "priority": "Urgent",
                "tags": bson::Bson::Null,
                "modify_date": bson::DateTime::now(),
                "order": 1000i64,
            })
            .unwrap();

        crate::database::migrations::migration_002_remove_duplicate_priority(&db.instance).unwrap();

        let tasks = db.get_tasks(ProjectEntry::All).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].priority, Priority::Urgent);
    }

    // --- Project tests ---

    #[test]
    fn create_and_retrieve_project() {
        let (_dir, db) = test_db();
        let project = make_project("Work");
        db.create_project(project.clone()).unwrap();

        let all = db.all_projects().unwrap();
        assert_eq!(all.len(), 1);
        match &all[0] {
            ProjectEntry::Project(p) => assert_eq!(p.name, "Work"),
            _ => panic!("expected Project variant"),
        }
    }

    #[test]
    fn one_project_returns_none_for_missing_id() {
        let (_dir, db) = test_db();
        let result = db.one_project(ObjectId::new()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn delete_project_cascades_tasks() {
        let (_dir, db) = test_db();
        let project = make_project("Work");
        let pid = db.create_project(project.clone()).unwrap();

        let task = make_task("write tests", Some(pid));
        db.create_task(task).unwrap();

        db.delete_project(pid).unwrap();

        assert!(db.all_projects().unwrap().is_empty());
        assert!(db.get_tasks(ProjectEntry::All).unwrap().is_empty());
    }

    // --- Task tests ---

    #[test]
    fn create_and_retrieve_task() {
        let (_dir, db) = test_db();
        let project = make_project("Work");
        let pid = db.create_project(project.clone()).unwrap();

        let project_entry = db.one_project(pid).unwrap().unwrap();
        let task = make_task("fix bug", Some(pid));
        db.create_task(task).unwrap();

        let tasks = db.get_tasks(project_entry).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "fix bug");
    }

    #[test]
    fn create_task_with_no_project() {
        let (_dir, db) = test_db();
        let task = make_task("inbox item", None);
        db.create_task(task).unwrap();

        let tasks = db.get_tasks(ProjectEntry::None).unwrap();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn get_tasks_all_returns_all_projects() {
        let (_dir, db) = test_db();
        let pid = db.create_project(make_project("Work")).unwrap();
        db.create_task(make_task("task A", Some(pid))).unwrap();
        db.create_task(make_task("task B", None)).unwrap();

        let all = db.get_tasks(ProjectEntry::All).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn update_task_persists_changes() {
        let (_dir, db) = test_db();
        let task = make_task("original", None);
        let id = task.id;
        db.create_task(task.clone()).unwrap();

        let mut updated = task;
        updated.title = "updated".to_string();
        db.update_task(updated).unwrap();

        let fetched = db.one_task(id).unwrap().unwrap();
        assert_eq!(fetched.title, "updated");
    }

    #[test]
    fn delete_task_returns_the_deleted_task() {
        let (_dir, db) = test_db();
        let task = make_task("doomed task", None);
        let id = task.id;
        db.create_task(task).unwrap();

        let deleted = db.delete_task(id).unwrap();
        assert_eq!(deleted.title, "doomed task");
        assert!(db.get_tasks(ProjectEntry::All).unwrap().is_empty());
    }

    #[test]
    fn tasks_returned_sorted_by_order() {
        let (_dir, db) = test_db();
        let mut t1 = make_task("first", None);
        let mut t2 = make_task("second", None);
        let mut t3 = make_task("third", None);
        t1.order = 3000;
        t2.order = 1000;
        t3.order = 2000;
        db.create_task(t1).unwrap();
        db.create_task(t2).unwrap();
        db.create_task(t3).unwrap();

        let tasks = db.get_tasks(ProjectEntry::All).unwrap();
        assert_eq!(tasks[0].title, "second");
        assert_eq!(tasks[1].title, "third");
        assert_eq!(tasks[2].title, "first");
    }

    // --- Undo/Redo tests ---

    #[test]
    fn undo_create_removes_task() {
        let (_dir, db) = test_db();
        db.create_task(make_task("temp", None)).unwrap();
        assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 1);

        db.undo().unwrap();
        assert!(db.get_tasks(ProjectEntry::All).unwrap().is_empty());
    }

    #[test]
    fn undo_delete_restores_task() {
        let (_dir, db) = test_db();
        let task = make_task("restore me", None);
        let id = task.id;
        db.create_task(task).unwrap();
        db.delete_task(id).unwrap();
        assert!(db.get_tasks(ProjectEntry::All).unwrap().is_empty());

        db.undo().unwrap();
        assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 1);
    }

    #[test]
    fn undo_update_reverts_to_original() {
        let (_dir, db) = test_db();
        let task = make_task("original", None);
        let id = task.id;
        db.create_task(task.clone()).unwrap();

        let mut updated = task;
        updated.title = "updated".to_string();
        db.update_task(updated).unwrap();

        db.undo().unwrap();
        let fetched = db.one_task(id).unwrap().unwrap();
        assert_eq!(fetched.title, "original");
    }

    #[test]
    fn redo_reapplies_undone_operation() {
        let (_dir, db) = test_db();
        db.create_task(make_task("redoable", None)).unwrap();
        db.undo().unwrap();
        assert!(db.get_tasks(ProjectEntry::All).unwrap().is_empty());

        db.redo().unwrap();
        assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 1);
    }

    #[test]
    fn new_operation_clears_redo_stack() {
        let (_dir, db) = test_db();
        db.create_task(make_task("first", None)).unwrap();
        db.undo().unwrap();
        // new operation should clear redo
        db.create_task(make_task("second", None)).unwrap();
        db.redo().unwrap();
        // redo stack is empty, so redo is a no-op — still just 1 task
        assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 1);
    }

    // --- App state tests ---

    #[test]
    fn save_and_restore_current_project() {
        let (_dir, db) = test_db();
        db.save_current_project(ProjectEntry::All).unwrap();
        let restored = db.get_recent_project();
        assert_eq!(restored, Some(ProjectEntry::All));
    }
}
