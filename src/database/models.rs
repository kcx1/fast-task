use std::fmt::Display;

use polodb_core::bson::{self, Document};
use polodb_core::bson::{Bson, DateTime, doc, oid::ObjectId};
use serde::{Deserialize, Serialize};

/// A named container for tasks. `tags` is reserved for future filtering.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Project {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub name: String,
    pub tags: Option<Vec<String>>,
}

impl Display for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl Project {
    pub fn new(name: &str, tags: Option<Vec<String>>) -> Self {
        Self {
            id: ObjectId::new(),
            name: name.to_string(),
            tags,
        }
    }

    pub fn load(self) -> LoadedProject {
        LoadedProject {
            id: Some(self.id),
            project: Some(self),
        }
    }
}

#[derive(Debug)]
pub struct LoadedProject {
    pub id: Option<ObjectId>,
    pub project: Option<Project>,
}

/// The current project filter: all tasks, no-project tasks, or a specific project.
#[derive(Eq, PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum ProjectEntry {
    All,
    None,
    Project(Project),
}

impl Display for ProjectEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let result = match self {
            ProjectEntry::All => "All",
            ProjectEntry::None => "None",
            ProjectEntry::Project(project) => &project.name,
        };
        write!(f, "{}", result)
    }
}

impl ProjectEntry {
    pub fn task_lookup(&self) -> Document {
        match self {
            Self::All => doc! {},
            Self::None => doc! {"project_id": bson::Bson::Null},
            Self::Project(project) => doc! {"project_id": project.id},
        }
    }

    pub fn get_id(&self) -> Option<ObjectId> {
        // To use for task creation
        match self {
            Self::Project(project) => Some(project.id),
            _ => None,
        }
    }
}

/// Lifecycle state of a task; drives row color, status icon, and default filter behavior.
#[derive(Default, PartialEq, Clone, Debug, Serialize, Deserialize)]
pub enum TaskStatus {
    #[default]
    NotStarted,
    InProgress,
    Completed,
    OnHold,
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            TaskStatus::NotStarted => "○",
            TaskStatus::InProgress => "◑",
            TaskStatus::Completed => "●",
            TaskStatus::OnHold => "⊘",
        };
        write!(f, "{}", s)
    }
}

/// The primary unit of work. `title` is stored as `single_line` in the DB for backward compat.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Task {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub project_id: Option<ObjectId>,
    #[serde(rename = "single_line")]
    pub title: String,
    pub details: String,
    #[serde(default)]
    pub code: bool,
    #[serde(default)]
    pub status: TaskStatus,
    pub due: Option<DateTime>,
    #[serde(default)]
    pub wait_until: Option<DateTime>,
    #[serde(alias = "priorty")]
    pub priority: Priority,
    pub tags: Option<Vec<String>>,
    pub modify_date: DateTime,
    pub order: u64,
    #[serde(default)]
    pub recurrence: Option<Recurrence>,
}

pub(crate) const ORDER_GAP: u64 = 1_000;

impl Task {
    pub fn edit(&mut self, contents: &str) {
        self.title = contents.to_string();
        self.modify_date = DateTime::now();
    }

    pub fn get_next_gap(&self) -> u64 {
        self.order + ORDER_GAP
    }
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: ObjectId::new(),
            project_id: None,
            title: String::new(),
            details: String::new(),
            code: false,
            status: TaskStatus::NotStarted,
            due: None,
            wait_until: None,
            priority: Priority::Normal,
            tags: None,
            modify_date: DateTime::now(),
            order: 0,
            recurrence: None,
        }
    }
}

/// How often a completed task auto-spawns its next occurrence.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Recurrence {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl std::fmt::Display for Recurrence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Daily => "Daily",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
            Self::Yearly => "Yearly",
        };
        write!(f, "{}", s)
    }
}

/// Task urgency level. Affects row color and icon in the task list.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Priority {
    Urgent,
    Normal,
    Low,
}

impl From<Priority> for Bson {
    fn from(p: Priority) -> Self {
        Bson::String(p.to_string())
    }
}

impl std::fmt::Display for Priority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Priority::Urgent => "Urgent",
            Priority::Normal => "Normal",
            Priority::Low => "Low",
        };
        write!(f, "{}", s)
    }
}

/// A timestamped note appended to a task; separate from the mutable `details` field.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Annotation {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub task_id: ObjectId,
    pub content: String,
    pub created_at: DateTime,
}

/// A single normalized tag stored in the tags collection.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub content: String,
}

impl Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}
