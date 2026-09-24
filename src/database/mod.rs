use crate::ui::app::FastTask;
use bson::oid::ObjectId;
use polodb_core::bson;

#[allow(clippy::module_inception)]
pub mod database;
pub mod history;
pub mod http_database;
pub mod migrations;
pub mod models;

pub use models::Annotation;
use models::Project;
pub use models::ProjectEntry;
pub use models::Task;

/// Core lifecycle: open and close the database connection.
pub trait Database {
    fn open() -> anyhow::Result<Self>
    where
        Self: Sized;

    fn close() -> anyhow::Result<()>;
}

/// Session persistence (not yet implemented).
pub trait SessionManagement {
    fn save_current_session(&self, app_state: FastTask) -> anyhow::Result<ObjectId>;
    fn get_previous_session(&self) -> anyhow::Result<Option<FastTask>>;
}

/// CRUD operations for projects.
pub trait ProjectManagement {
    fn all_projects(&self) -> anyhow::Result<Vec<ProjectEntry>>;
    fn one_project(&self, project_id: ObjectId) -> anyhow::Result<Option<ProjectEntry>>;
    fn create_project(&self, project: Project) -> anyhow::Result<ObjectId>;
    fn delete_project(&self, project_id: ObjectId) -> anyhow::Result<()>;
    fn update_project(&self, project: Project) -> anyhow::Result<ObjectId>;
}

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
}

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
