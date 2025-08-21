use fast_task::database::database::Db;
use fast_task::database::models::Project;
use fast_task::database::models::{Priority, Task, TaskStatus};
use fast_task::database::{ProjectEntry, ProjectManagement, TaskManagement};
use tempfile::TempDir;

fn test_db() -> (TempDir, Db) {
    let dir = TempDir::new().unwrap();
    let db = Db::open_path(dir.path().join("test.db")).unwrap();
    (dir, db)
}

fn make_task(single_line: &str, project_id: Option<polodb_core::bson::oid::ObjectId>) -> Task {
    Task {
        title: single_line.to_string(),
        project_id,
        order: 1000,
        ..Default::default()
    }
}

#[test]
fn full_task_lifecycle() {
    let (_dir, db) = test_db();

    // Create
    let task = make_task("write integration tests", None);
    let id = task.id;
    db.create_task(task).unwrap();

    let tasks = db.get_tasks(ProjectEntry::All).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "write integration tests");

    // Edit
    let mut task = db.one_task(id).unwrap().unwrap();
    task.title = "write integration tests (done)".to_string();
    db.update_task(task).unwrap();

    let fetched = db.one_task(id).unwrap().unwrap();
    assert_eq!(fetched.title, "write integration tests (done)");

    // Complete
    let mut task = db.one_task(id).unwrap().unwrap();
    task.status = TaskStatus::Completed;
    db.update_task(task).unwrap();

    let fetched = db.one_task(id).unwrap().unwrap();
    assert_eq!(fetched.status, TaskStatus::Completed);

    // Undo complete
    db.undo().unwrap();
    let fetched = db.one_task(id).unwrap().unwrap();
    assert_eq!(fetched.status, TaskStatus::NotStarted);
}

#[test]
fn project_lifecycle_with_cascade() {
    let (_dir, db) = test_db();

    // Create project
    let project = Project::new("Home", None);
    let pid = db.create_project(project).unwrap();

    // Add tasks
    db.create_task(make_task("mow lawn", Some(pid))).unwrap();
    db.create_task(make_task("fix fence", Some(pid))).unwrap();

    let project_entry = db.one_project(pid).unwrap().unwrap();
    let tasks = db.get_tasks(project_entry).unwrap();
    assert_eq!(tasks.len(), 2);

    // Delete project (should cascade to tasks)
    db.delete_project(pid).unwrap();

    assert!(db.all_projects().unwrap().is_empty());
    assert!(db.get_tasks(ProjectEntry::All).unwrap().is_empty());
}

#[test]
fn full_undo_redo_chain() {
    let (_dir, db) = test_db();

    // Three creates
    db.create_task(make_task("task A", None)).unwrap();
    db.create_task(make_task("task B", None)).unwrap();
    db.create_task(make_task("task C", None)).unwrap();
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 3);

    // Undo all three
    db.undo().unwrap();
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 2);
    db.undo().unwrap();
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 1);
    db.undo().unwrap();
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 0);

    // Redo all three
    db.redo().unwrap();
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 1);
    db.redo().unwrap();
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 2);
    db.redo().unwrap();
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 3);
}

#[test]
fn multiple_projects_isolated() {
    let (_dir, db) = test_db();

    let pid_work = db.create_project(Project::new("Work", None)).unwrap();
    let pid_home = db.create_project(Project::new("Home", None)).unwrap();

    db.create_task(make_task("write report", Some(pid_work)))
        .unwrap();
    db.create_task(make_task("mow lawn", Some(pid_home)))
        .unwrap();
    db.create_task(make_task("inbox item", None)).unwrap();

    let work_entry = db.one_project(pid_work).unwrap().unwrap();
    let home_entry = db.one_project(pid_home).unwrap().unwrap();

    assert_eq!(db.get_tasks(work_entry).unwrap().len(), 1);
    assert_eq!(db.get_tasks(home_entry).unwrap().len(), 1);
    assert_eq!(db.get_tasks(ProjectEntry::None).unwrap().len(), 1);
    assert_eq!(db.get_tasks(ProjectEntry::All).unwrap().len(), 3);
}

#[test]
fn task_priority_roundtrip() {
    let (_dir, db) = test_db();

    let task = Task {
        title: "urgent task".to_string(),
        priority: Priority::Urgent,
        order: 1000,
        ..Default::default()
    };
    let id = task.id;
    db.create_task(task).unwrap();

    let fetched = db.one_task(id).unwrap().unwrap();
    assert_eq!(fetched.priority, Priority::Urgent);
}
