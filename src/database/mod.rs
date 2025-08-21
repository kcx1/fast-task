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
}

/// Operations for the normalized tag store.
pub trait TagManagement {
    fn all_tags(&self) -> anyhow::Result<Vec<String>>;
    fn upsert_tags(&self, tags: &[String]) -> anyhow::Result<()>;
}
