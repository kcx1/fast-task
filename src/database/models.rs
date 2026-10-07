use std::fmt::Display;

use bson::Document;
use bson::{Bson, DateTime, doc, oid::ObjectId};
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
    /// Syntax-highlight `details` as this language (implies `code`).
    #[serde(default)]
    pub language: Option<CodeLanguage>,
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

    /// The next instance of a recurring task, due one period after its due date
    /// (or today, if it has none). `None` if it doesn't recur.
    pub fn next_occurrence(&self) -> Option<Task> {
        let recurrence = self.recurrence.as_ref()?;
        let base = self
            .due
            .as_ref()
            .and_then(bson_dt_to_jiff_date)
            .unwrap_or_else(|| jiff::Zoned::now().date());
        let span = match recurrence {
            Recurrence::Daily => jiff::Span::new().days(1i64),
            Recurrence::Weekly => jiff::Span::new().weeks(1i64),
            Recurrence::Monthly => jiff::Span::new().months(1i64),
            Recurrence::Yearly => jiff::Span::new().years(1i64),
        };
        let next_date = base.checked_add(span).ok()?;
        Some(Task {
            title: self.title.clone(),
            details: self.details.clone(),
            priority: self.priority.clone(),
            tags: self.tags.clone(),
            code: self.code,
            language: self.language,
            project_id: self.project_id,
            recurrence: self.recurrence.clone(),
            wait_until: self.wait_until,
            due: from_jiff_to_datetime(next_date),
            order: self.get_next_gap(),
            ..Default::default()
        })
    }
}

/// Converts a BSON `DateTime` to a `jiff` civil date in UTC.
pub fn bson_dt_to_jiff_date(dt: &DateTime) -> Option<jiff::civil::Date> {
    let ts = jiff::Timestamp::from_millisecond(dt.timestamp_millis()).ok()?;
    Some(ts.to_zoned(jiff::tz::TimeZone::UTC).date())
}

/// Converts a `jiff` civil date to a BSON `DateTime` at midnight UTC.
pub fn from_jiff_to_datetime(dt: jiff::civil::Date) -> Option<DateTime> {
    DateTime::builder()
        .year(dt.year() as i32)
        .month(dt.month() as u8)
        .day(dt.day() as u8)
        .build()
        .ok()
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
            language: None,
        }
    }
}

/// Language used to syntax-highlight a task's details.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLanguage {
    Rust,
    Python,
    Lua,
    JavaScript,
    Css,
    Shell,
    Json,
    Yaml,
    Sql,
    Markdown,
    Html,
}

impl CodeLanguage {
    pub const ALL: [CodeLanguage; 11] = [
        Self::Rust,
        Self::Python,
        Self::Lua,
        Self::JavaScript,
        Self::Css,
        Self::Shell,
        Self::Json,
        Self::Yaml,
        Self::Sql,
        Self::Markdown,
        Self::Html,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Lua => "Lua",
            Self::JavaScript => "JavaScript",
            Self::Css => "CSS",
            Self::Shell => "Shell",
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Sql => "SQL",
            Self::Markdown => "Markdown",
            Self::Html => "HTML",
        }
    }

    /// File extension syntect resolves to the right syntax definition.
    pub fn syntax_key(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::Python => "py",
            Self::Lua => "lua",
            Self::JavaScript => "js",
            Self::Css => "css",
            Self::Shell => "sh",
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Sql => "sql",
            Self::Markdown => "md",
            Self::Html => "html",
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
    /// Who wrote it, when known: browser notes carry the editor's name;
    /// desktop notes don't (nobody is asked for a name there).
    #[serde(default)]
    pub author: Option<String>,
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

/// One line of the LAN share's activity log (see `database::activity`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ActivityEntry {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub at: DateTime,
    /// The browser user's name; `None` for the desktop.
    pub who: Option<String>,
    /// The whole change as one sentence, e.g. `completed “Fix the tap”`. Entries
    /// from before `action` existed have only this.
    pub what: String,
    /// The kind of change, for the log's colored chip.
    #[serde(default)]
    pub action: Option<ActivityAction>,
    /// What it happened to — usually a task title.
    #[serde(default)]
    pub subject: Option<String>,
    /// Anything else worth a line, e.g. the old title or the note's text.
    #[serde(default)]
    pub detail: Option<String>,
}

/// The kinds of change the activity log shows.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityAction {
    Added,
    Completed,
    Started,
    Reopened,
    OnHold,
    Renamed,
    Edited,
    Priority,
    Moved,
    Deleted,
    Noted,
    NoteDeleted,
    Undid,
    Redid,
}

impl ActivityAction {
    /// Past-tense verb for the one-line sentence (`what`).
    pub fn verb(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Completed => "completed",
            Self::Started => "started",
            Self::Reopened => "reopened",
            Self::OnHold => "put on hold",
            Self::Renamed => "renamed",
            Self::Edited => "edited",
            Self::Priority => "changed the priority of",
            Self::Moved => "moved",
            Self::Deleted => "deleted",
            Self::Noted => "added a note to",
            Self::NoteDeleted => "deleted a note on",
            Self::Undid => "undid a change to",
            Self::Redid => "redid a change to",
        }
    }
}
