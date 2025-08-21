use polodb_core::bson::{self, oid::ObjectId};
use serde::{Deserialize, Serialize};

use crate::database::{Task, models::Project};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum SaveableItem {
    Task(Task),
    Project(Project),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Event {
    Create(SaveableItem),
    Update {
        before: SaveableItem,
        after: SaveableItem,
    },
    Delete(SaveableItem),
}

#[derive(Debug)]
pub enum HistoryAction {
    Append(Event),
    Undo(Event),
    Redo(Event),
}

impl HistoryAction {
    pub fn append(event: Event) -> Self {
        HistoryAction::Append(event)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRecord {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub timestamp: bson::DateTime,
    pub event: Event,
}
