#[cfg(not(target_arch = "wasm32"))]
use crate::ui::app::FastTask;
#[cfg(not(target_arch = "wasm32"))]
use bson::oid::ObjectId;

#[cfg(not(target_arch = "wasm32"))]
pub mod activity;
#[allow(clippy::module_inception)]
#[cfg(not(target_arch = "wasm32"))]
pub mod database;
#[cfg(not(target_arch = "wasm32"))]
pub mod history;
#[cfg(not(target_arch = "wasm32"))]
pub mod migrations;
// Plain data types, shared with the browser client.
pub mod models;

pub use models::Annotation;
#[cfg(not(target_arch = "wasm32"))]
use models::Project;
pub use models::ProjectEntry;
pub use models::Task;

#[cfg(not(target_arch = "wasm32"))]
/// Core lifecycle: open and close the database connection.
pub trait Database {
    fn open() -> anyhow::Result<Self>
    where
        Self: Sized;

    fn close() -> anyhow::Result<()>;
}

#[cfg(not(target_arch = "wasm32"))]
/// Session persistence (not yet implemented).
pub trait SessionManagement {
    fn save_current_session(&self, app_state: FastTask) -> anyhow::Result<ObjectId>;
    fn get_previous_session(&self) -> anyhow::Result<Option<FastTask>>;
}

#[cfg(not(target_arch = "wasm32"))]
/// CRUD operations for projects.
pub trait ProjectManagement {
    fn all_projects(&self) -> anyhow::Result<Vec<ProjectEntry>>;
    fn one_project(&self, project_id: ObjectId) -> anyhow::Result<Option<ProjectEntry>>;
    fn create_project(&self, project: Project) -> anyhow::Result<ObjectId>;
    fn delete_project(&self, project_id: ObjectId) -> anyhow::Result<()>;
    fn update_project(&self, project: Project) -> anyhow::Result<ObjectId>;
}

#[cfg(not(target_arch = "wasm32"))]
/// CRUD operations for tasks; the primary backend seam for dependency injection.
pub trait TaskManagement {
    fn one_task(&self, task_id: ObjectId) -> anyhow::Result<Option<Task>>;
    fn get_tasks(&self, lookup: ProjectEntry) -> anyhow::Result<Vec<Task>>;
    fn delete_task(&self, task_id: ObjectId) -> anyhow::Result<Task>;
    fn update_task(&self, task: Task) -> anyhow::Result<ObjectId>;
    fn create_task(&self, task: Task) -> anyhow::Result<ObjectId>;
    /// Read-modify-write: re-read the task, apply `f`, save it. Use this instead
    /// of `update_task` with a UI-side copy, which can be stale and would silently
    /// revert a change saved in between. `Ok(None)` if the task is gone.
    ///
    /// The default is not atomic; backends that can should override it (`Db`
    /// does, under its write lock).
    fn modify_task(
        &self,
        task_id: ObjectId,
        f: &mut dyn FnMut(&mut Task),
    ) -> anyhow::Result<Option<ObjectId>> {
        let Some(mut task) = self.one_task(task_id)? else {
            return Ok(None);
        };
        f(&mut task);
        self.update_task(task).map(Some)
    }

    /// Set a task's status (atomically, via `modify_task`). Completing a
    /// recurring task that wasn't already completed also creates its next
    /// occurrence. Every status change — desktop single / bulk, share API —
    /// goes through here so recurrence is handled the same way.
    fn set_status(
        &self,
        task_id: ObjectId,
        status: &models::TaskStatus,
    ) -> anyhow::Result<Option<ObjectId>> {
        use models::TaskStatus;
        let mut just_completed: Option<Task> = None;
        let result = self.modify_task(task_id, &mut |task| {
            if *status == TaskStatus::Completed
                && task.status != TaskStatus::Completed
                && task.recurrence.is_some()
            {
                just_completed = Some(task.clone());
            }
            task.status = status.clone();
        })?;
        if let Some(next) = just_completed.and_then(|t| t.next_occurrence()) {
            self.create_task(next)?;
        }
        Ok(result)
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Operations for the normalized tag store.
pub trait TagManagement {
    fn all_tags(&self) -> anyhow::Result<Vec<String>>;
    fn upsert_tags(&self, tags: &[String]) -> anyhow::Result<()>;
    /// Every tag (stored or in use on a task) with how many tasks carry it, sorted by name.
    fn tag_usage(&self) -> anyhow::Result<Vec<(String, usize)>>;
    /// Rename `from` to `to` in the tag store and on every task that has it.
    /// Returns the number of tasks changed.
    fn rename_tag(&self, from: &str, to: &str) -> anyhow::Result<usize>;
    /// Remove `name` from the tag store and from every task that has it.
    /// Returns the number of tasks changed.
    fn delete_tag(&self, name: &str) -> anyhow::Result<usize>;
}
