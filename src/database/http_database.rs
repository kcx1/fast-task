use polodb_core::bson::oid::ObjectId;
use reqwest::StatusCode;
use reqwest::blocking::Client;

use crate::database::ProjectEntry;
use crate::database::Task;
use crate::database::TaskManagement;

pub struct HttpDatabase {
    client: Client,
    base_url: String,
}

impl TaskManagement for HttpDatabase {
    fn one_task(&self, task_id: ObjectId) -> anyhow::Result<Option<Task>> {
        let url = format!("{}/task/{}", self.base_url, task_id);

        let res = self.client.get(url).send()?;

        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        Ok(Some(res.json()?))
    }

    fn get_tasks(&self, lookup: ProjectEntry) -> anyhow::Result<Vec<Task>> {
        let id = match lookup {
            ProjectEntry::All => "All".to_string(),
            ProjectEntry::None => "None".to_string(),
            ProjectEntry::Project(project) => project.id.to_string(),
        };
        let url = format!("{}/tasks?project_id={id}", self.base_url,);
        let res = self.client.get(url).send()?;
        Ok(res.json()?)
    }

    fn delete_task(&self, task_id: ObjectId) -> anyhow::Result<Task> {
        let url = format!("{}/task/{}", self.base_url, task_id);
        let task = self.one_task(task_id)?;
        let res = self.client.delete(url).send()?;
        if res.status().is_success()
            && let Some(t) = task
        {
            return Ok(t);
        }
        Err(anyhow::anyhow!("Error deleting task"))
    }

    fn update_task(&self, task: Task) -> anyhow::Result<ObjectId> {
        let url = format!("{}/task/{}", self.base_url, task.id);
        let res = self.client.patch(url).json(&task).send()?;
        Ok(res.json()?)
    }

    fn create_task(&self, task: Task) -> anyhow::Result<ObjectId> {
        // let id = match task.project_id {
        //     ProjectEntry::All => "All".to_string(),
        //     ProjectEntry::None => "None".to_string(),
        //     ProjectEntry::Project(project) => project.id.to_string(),
        // };
        let url = format!("{}/task?project_id={:?}", self.base_url, task.project_id);
        let res = self.client.post(url).json(&task).send()?;
        Ok(res.json()?)
    }
}
