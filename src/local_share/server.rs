use anyhow::anyhow;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Router, routing::get};
use polodb_core::bson::oid::ObjectId;
use std::path::PathBuf;

use crate::database::{ProjectEntry, ProjectManagement, Task, TaskManagement};
use crate::ui::app::DB;

const WEB_ROOT: &str = "src/local_share/web";

async fn hello() -> &'static str {
    "hello"
}

async fn index() -> impl IntoResponse {
    serve_file("index.html")
}

async fn js() -> impl IntoResponse {
    serve_file("app.js")
}

async fn wasm() -> impl IntoResponse {
    serve_file("app_bg.wasm")
}

fn serve_file(name: &str) -> Response<Body> {
    let mut path = PathBuf::from(WEB_ROOT);
    path.push(name);

    match std::fs::read(path) {
        Ok(bytes) => {
            let mime = match name {
                "app.js" => "text/javascript",
                "app_bg.wasm" => "application/wasm",
                _ => "text/html",
            };

            Response::builder()
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(bytes))
                .unwrap()
        }
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404"))
            .unwrap(),
    }
}

fn resolve_project_id(project_id: Query<String>) -> anyhow::Result<ProjectEntry> {
    match ObjectId::parse_str(project_id.as_str()) {
        Ok(id) => match DB.one_project(id) {
            Ok(Some(project)) => Ok(project),
            _ => Err(anyhow!("Project Not Found")),
        },

        Err(_) => match project_id.as_str() {
            "All" => Ok(ProjectEntry::All),
            "None" => Ok(ProjectEntry::None),
            _ => Err(anyhow!("Unknown Parameter")),
        },
    }
}

async fn get_one_task(task_id: Path<ObjectId>) -> impl IntoResponse {
    let task = DB.one_task(*task_id);
    Json(task.expect("Problem parsing task into JSON"))
}

async fn get_all_tasks(project_id: Query<String>) -> impl IntoResponse {
    match resolve_project_id(project_id) {
        Ok(project_entry) => Json(
            DB.get_tasks(project_entry)
                .expect("Problem Parsing tasks into JSON"),
        ),
        Err(_) => todo!("Send an Error Json"),
    }
}

async fn create_task(_task: Json<Task>) -> impl IntoResponse {
    // match resolve_project_id(project_id) {
    //     Ok(project_entry) => Json(DB.create_task(Task::from(task), project_entry)),
    //     Err(_) => todo!("Figure Out how to handle this!"),
    // }
}

async fn update_task(_task_id: Path<ObjectId>, _task: Json<Task>) -> impl IntoResponse {
    // implement your logic to update a task here
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from("Task updated successfully"))
        .unwrap()
}

async fn delete_task(_task_id: Path<ObjectId>) -> impl IntoResponse {
    // implement your logic to delete a task here
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::from(""))
        .unwrap()
}

pub fn start_server() {
    tokio::spawn(async {
        let server = Router::new()
            .route("/hello", get(hello))
            .route("/", get(index))
            .route("/app.js", get(js))
            .route("/app_bg.wasm", get(wasm))
            .route("/task", post(create_task))
            .route(
                "/task/:task_id",
                get(get_one_task).patch(update_task).delete(delete_task),
            )
            .route("/tasks", get(get_all_tasks));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
            .await
            .unwrap();

        axum::serve(listener, server).await.unwrap();
    });
}
