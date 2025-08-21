use anyhow::Context;
use polodb_core::CollectionT;
use polodb_core::bson::{Document, doc};

pub(crate) const CURRENT_SCHEMA_VERSION: u32 = 6;

const TASK_COLLECTION: &str = "tasks";
const HISTORY_COLLECTION: &str = "history";
const META_COLLECTION: &str = "meta";
const SCHEMA_VERSION_KEY: &str = "schema_version";

pub(crate) fn run(db: &polodb_core::Database) -> anyhow::Result<()> {
    let version = get_schema_version(db);
    if version < 1 {
        migration_001_fix_priority_typo(db)?;
    }
    if version < 2 {
        migration_002_remove_duplicate_priority(db)?;
    }
    if version < 3 {
        migration_003_create_tags_collection(db)?;
    }
    // v4: adds wait_until (Option<DateTime>) — no-op, serde(default) handles missing field
    // v5: adds recurrence (Option<Recurrence>) — no-op, serde(default) handles missing field
    // v6: annotations collection — no-op, created on first insert
    if version < CURRENT_SCHEMA_VERSION {
        set_schema_version(db, CURRENT_SCHEMA_VERSION);
    }
    Ok(())
}

pub(crate) fn migration_003_create_tags_collection(
    _db: &polodb_core::Database,
) -> anyhow::Result<()> {
    // Tags collection is created implicitly on first insert; nothing to rewrite.
    Ok(())
}

fn get_schema_version(db: &polodb_core::Database) -> u32 {
    db.collection::<Document>(META_COLLECTION)
        .find_one(doc! { "key": SCHEMA_VERSION_KEY })
        .ok()
        .flatten()
        .and_then(|d| d.get_i32("version").ok())
        .map(|v| v as u32)
        .unwrap_or(0)
}

fn set_schema_version(db: &polodb_core::Database, version: u32) {
    let col = db.collection::<Document>(META_COLLECTION);
    let existing = col
        .find_one(doc! { "key": SCHEMA_VERSION_KEY })
        .ok()
        .flatten();
    if existing.is_some() {
        col.update_one(
            doc! { "key": SCHEMA_VERSION_KEY },
            doc! { "$set": { "version": version as i32 } },
        )
        .ok();
    } else {
        col.insert_one(doc! { "key": SCHEMA_VERSION_KEY, "version": version as i32 })
            .ok();
    }
}

pub(crate) fn migration_001_fix_priority_typo(db: &polodb_core::Database) -> anyhow::Result<()> {
    let col = db.collection::<Document>(TASK_COLLECTION);
    let tasks: Vec<Document> = col
        .find(doc! {})
        .run()
        .context("Migration 001: find tasks")?
        .flatten()
        .collect();

    for task in tasks {
        if task.contains_key("priorty") && !task.contains_key("priority") {
            let id = task.get_object_id("_id")?;
            let val = task.get("priorty").unwrap().clone();
            col.update_one(
                doc! { "_id": id },
                doc! {
                    "$set": { "priority": val },
                    "$unset": { "priorty": "" }
                },
            )?;
        }
    }
    Ok(())
}

pub(crate) fn migration_002_remove_duplicate_priority(
    db: &polodb_core::Database,
) -> anyhow::Result<()> {
    let task_col = db.collection::<Document>(TASK_COLLECTION);
    let tasks: Vec<Document> = task_col
        .find(doc! {})
        .run()
        .context("Migration 002: find tasks")?
        .flatten()
        .collect();

    for task in tasks {
        if task.contains_key("priorty") {
            let id = task.get_object_id("_id")?;
            if task.contains_key("priority") {
                task_col.update_one(doc! { "_id": id }, doc! { "$unset": { "priorty": "" } })?;
            } else {
                let val = task.get("priorty").unwrap().clone();
                task_col.update_one(
                    doc! { "_id": id },
                    doc! {
                        "$set": { "priority": val },
                        "$unset": { "priorty": "" }
                    },
                )?;
            }
        }
    }

    db.collection::<Document>(HISTORY_COLLECTION)
        .delete_many(doc! {})
        .context("Migration 002: clear history")?;

    Ok(())
}
