//! Serve the LAN share's web client over a throwaway, seeded database, so the
//! browser view can be tried without touching your real tasks:
//!
//!     scripts/build-web.sh && cargo run --example share_demo
//!
//! Prints the URL, then "Robin" adds a task every 10 s so you can watch live
//! updates and the activity log.
//! Loopback only; Ctrl+C to stop.

use bson::DateTime;
use fast_task::database::database::Db;
use fast_task::database::models::{
    Annotation, CodeLanguage, Priority, Project, ProjectEntry, Recurrence, Task, TaskStatus,
};
use fast_task::database::{ProjectManagement, TaskManagement};

fn main() -> anyhow::Result<()> {
    let dir = tempfile::TempDir::new()?;
    let db = Db::open_path(dir.path().join("demo.db"))?;

    let project = Project::new("Home", None);
    db.create_project(project.clone())?;
    db.save_current_project(ProjectEntry::Project(project.clone()))?;
    let day = 24 * 60 * 60 * 1000;
    let now = DateTime::now().timestamp_millis();
    let tasks = [
        Task {
            title: "Fix the kitchen tap".into(),
            details: "Washer is worn — 15mm, from the hardware store.".into(),
            priority: Priority::Urgent,
            due: Some(DateTime::from_millis(now - day)),
            tags: Some(vec!["kitchen".into()]),
            ..Default::default()
        },
        Task {
            title: "Water the plants".into(),
            status: TaskStatus::InProgress,
            due: Some(DateTime::from_millis(now)),
            recurrence: Some(Recurrence::Weekly),
            ..Default::default()
        },
        Task {
            title: "Script to rename photos".into(),
            details: "fn main() {\n    for f in std::fs::read_dir(\".\").unwrap() {\n        println!(\"{:?}\", f);\n    }\n}".into(),
            code: true,
            language: Some(CodeLanguage::Rust),
            priority: Priority::Low,
            due: Some(DateTime::from_millis(now + 5 * day)),
            ..Default::default()
        },
        Task {
            title: "Call the plumber".into(),
            status: TaskStatus::OnHold,
            ..Default::default()
        },
        Task {
            title: "Buy light bulbs".into(),
            status: TaskStatus::Completed,
            ..Default::default()
        },
    ];
    let mut first = None;
    for (i, task) in tasks.into_iter().enumerate() {
        let id = db.create_task(Task {
            project_id: Some(project.id),
            order: (i as u64 + 1) * 1000,
            ..task
        })?;
        first.get_or_insert(id);
    }
    db.add_annotation(Annotation {
        id: bson::oid::ObjectId::new(),
        task_id: first.unwrap(),
        content: "Checked under the sink — it's the cold side.".into(),
        created_at: DateTime::now(),
        author: None,
    })?;

    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let share =
        fast_task::local_share::server::start_on(db.clone(), listener, std::sync::Arc::new(|| {}))?;
    // A playground: let the browser edit right away.
    share.set_allow_edits(true);
    db.set_activity_logging(true);
    let url = share.url().replace(
        &share.display_addr(),
        &format!("127.0.0.1:{}", share.port()),
    );
    println!("{url}");

    for n in 1.. {
        std::thread::sleep(std::time::Duration::from_secs(10));
        fast_task::database::activity::with_actor("Robin", || {
            db.create_task(Task {
                title: format!("Added live #{n}"),
                project_id: Some(project.id),
                order: (10 + n) * 1000,
                ..Default::default()
            })
        })?;
    }
    Ok(())
}
