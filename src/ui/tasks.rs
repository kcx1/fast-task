use anyhow::Context;
use egui::InnerResponse;
use egui::Key;
use egui::Sense;
use jiff::civil::Date;
use polodb_core::bson::DateTime;
use polodb_core::bson::oid::ObjectId;

use crate::database::TaskManagement;
use crate::database::models::{ORDER_GAP, Priority, Recurrence, TaskStatus};
use crate::database::{ProjectEntry, Task};
use crate::ui::app::EditFocus;
use crate::ui::app::FastTask;
use crate::ui::app::Mode;
use crate::ui::app::UpdateMessage;
use crate::ui::app::WindowState;

/// Shared reference to any `TaskManagement` implementation; passed to all background ops.
type Backend = std::sync::Arc<dyn TaskManagement + Send + Sync>;

fn parse_tags(buf: &str) -> Option<Vec<String>> {
    if buf.is_empty() {
        return None;
    }
    let tags: Vec<String> = buf
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if tags.is_empty() { None } else { Some(tags) }
}

/// How the task list is ordered. `Free` preserves the user-defined drag order.
#[derive(Default, PartialEq, Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum SortOrder {
    #[default]
    Free,
    DueDate,
    Modified,
    Status,
    Tags,
}

impl SortOrder {
    /// Human-readable label shown in the sort picker and status bar.
    pub fn label(&self) -> &'static str {
        match self {
            SortOrder::Free => "Free",
            SortOrder::DueDate => "Due Date",
            SortOrder::Modified => "Modified",
            SortOrder::Status => "Status",
            SortOrder::Tags => "Tags",
        }
    }
}

/// Mutable working copy of a task's fields while the editor is open.
/// Flushed back to a `Task` on save, or discarded on Esc.
#[derive(Clone)]
pub struct TaskWriter {
    pub title_buffer: String,
    pub details_buffer: String,
    pub tags_buffer: String,
    pub duedate: Option<DateTime>,
    pub due_text: String,
    pub wait_until: Option<DateTime>,
    pub wait_text: String,
    pub priority: Priority,
    pub status: TaskStatus,
    pub code: bool,
    pub recurrence: Option<Recurrence>,
    pub has_focus: EditFocus,
    pub order: u64,
    pub initial_frame: bool,
}

/// Converts a `jiff` civil date to a BSON `DateTime` at midnight UTC.
pub fn from_jiff_to_datetime(dt: Date) -> Option<DateTime> {
    DateTime::builder()
        .year(dt.year() as i32)
        .month(dt.month() as u8)
        .day(dt.day() as u8)
        .build()
        .ok()
}

impl Default for TaskWriter {
    fn default() -> Self {
        Self {
            title_buffer: Default::default(),
            details_buffer: Default::default(),
            tags_buffer: Default::default(),
            due_text: Default::default(),
            wait_until: None,
            wait_text: Default::default(),
            code: Default::default(),
            recurrence: None,
            status: TaskStatus::NotStarted,
            duedate: None,
            priority: Priority::Normal,
            has_focus: Default::default(),
            order: Default::default(),
            initial_frame: true,
        }
    }
}

impl TaskWriter {
    pub fn flush(&mut self) {
        self.title_buffer.clear();
        self.details_buffer.clear();
        self.tags_buffer.clear();
        self.due_text.clear();
        self.duedate = None;
        self.wait_text.clear();
        self.wait_until = None;
        self.priority = Priority::Normal;
        self.status = TaskStatus::NotStarted;
        self.recurrence = None;
        self.has_focus = Default::default();
        self.initial_frame = true;
    }
}

impl From<Task> for TaskWriter {
    fn from(value: Task) -> Self {
        Self {
            title_buffer: value.title,
            details_buffer: value.details,
            code: value.code,
            recurrence: value.recurrence,
            status: value.status,
            tags_buffer: value.tags.unwrap_or_default().join(", "),
            duedate: value.due,
            due_text: String::new(),
            wait_until: value.wait_until,
            wait_text: String::new(),
            priority: value.priority,
            order: value.order,
            has_focus: Default::default(),
            initial_frame: false,
        }
    }
}

/// All per-frame task-pane state: the loaded list, cursor, filters, pickers, and editor draft.
#[derive(Default)]
pub struct TaskManager {
    pub tasks: Vec<Task>,
    pub current: Option<usize>,
    pub writer: TaskWriter,
    pub selected_tasks: std::collections::HashSet<ObjectId>,
    pub clipboard_task: Option<Task>,
    pub filter_query: String,
    pub filter_open: bool,
    pub filter_just_opened: bool,
    pub show_completed: bool,
    pub status_picker_open: bool,
    pub status_picker_task_id: Option<ObjectId>,
    pub status_picker_ids: Vec<ObjectId>,
    pub status_picker_cursor: usize,
    pub sort_order: SortOrder,
    pub sort_picker_open: bool,
    pub sort_picker_cursor: usize,
    /// Highlighted tag autocomplete suggestion in the editor (keyboard nav).
    pub tag_suggestion_idx: Option<usize>,
}

impl TaskManager {
    /// Indices into `self.tasks` that match the current filter query.
    /// Returns all indices when the query is empty.
    pub fn visible_indices(&self) -> Vec<usize> {
        let now_ms = polodb_core::bson::DateTime::now().timestamp_millis();
        let q = self.filter_query.to_lowercase();
        self.tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                // Hide tasks whose wait_until is in the future
                if let Some(wait) = &t.wait_until
                    && wait.timestamp_millis() > now_ms
                {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                t.title.to_lowercase().contains(&q)
                    || t.priority.to_string().to_lowercase().contains(&q)
                    || t.tags
                        .as_deref()
                        .unwrap_or(&[])
                        .iter()
                        .any(|tag| tag.to_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Translate a visible-list cursor position to a real `tasks[]` index.
    pub fn real_index(&self, visible: usize) -> Option<usize> {
        self.visible_indices().get(visible).copied()
    }

    /// Returns a clone of the task at the current cursor position, if any.
    pub fn get_current_task(&self) -> Option<Task> {
        self.current
            .and_then(|i| self.real_index(i))
            .and_then(|ri| self.tasks.get(ri))
            .cloned()
    }
}

/// Renders the task-list pane, handles keybinds, and drives modal pickers.
pub fn task_state(ui: &mut egui::Ui, app: &mut FastTask) -> InnerResponse<()> {
    // Reset project hover every frame we're in Tasks so Projects pane starts from active filter
    app.project_manager.sync_hover();

    // Compute visible indices once per frame; reused by table, keybinds, and cursor clamp.
    let visible = app.task_manager.visible_indices();
    let vis_len = visible.len();

    // Clamp cursor to visible list length every frame
    app.task_manager.current = match app.task_manager.current {
        _ if vis_len == 0 => None,
        Some(i) if i >= vis_len => Some(vis_len - 1),
        other => other,
    };

    egui::CentralPanel::default().show_inside(ui, |ui| {
        // Bottom detail pane — must be added before content so egui reserves space correctly
        if app.app_state.show_detail_pane {
            let current_task = app
                .task_manager
                .current
                .and_then(|i| visible.get(i).copied())
                .and_then(|ri| app.task_manager.tasks.get(ri))
                .cloned();
            let edit_id = current_task.as_ref().map(|t| t.id);
            let mut edit_clicked = false;
            egui::Panel::bottom("task_detail_bottom")
                .resizable(true)
                .default_size(160.0)
                .show_inside(ui, |ui| {
                    edit_clicked = detail_panel(ui, current_task);
                });
            if edit_clicked && let Some(id) = edit_id {
                app.app_state.mode = Mode::Insert(Some(id));
                app.app_state.window_state = WindowState::Info;
            }
        }

        crate::ui::widgets::common::heading(ui, "Tasks");

        let mut pairs: Vec<(usize, &Task)> = visible
            .iter()
            .filter_map(|&ri| app.task_manager.tasks.get(ri).map(|t| (ri, t)))
            .collect();
        match app.task_manager.sort_order {
            SortOrder::Free => {}
            SortOrder::DueDate => pairs.sort_by(|(_, a), (_, b)| {
                let ta = a.due.as_ref().map(|d| d.timestamp_millis());
                let tb = b.due.as_ref().map(|d| d.timestamp_millis());
                match (ta, tb) {
                    (None, None) => std::cmp::Ordering::Equal,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (Some(a), Some(b)) => a.cmp(&b),
                }
            }),
            SortOrder::Modified => pairs.sort_by(|(_, a), (_, b)| {
                b.modify_date
                    .timestamp_millis()
                    .cmp(&a.modify_date.timestamp_millis())
            }),
            SortOrder::Status => pairs.sort_by_key(|(_, t)| match t.status {
                TaskStatus::InProgress => 0,
                TaskStatus::NotStarted => 1,
                TaskStatus::OnHold => 2,
                TaskStatus::Completed => 3,
            }),
            SortOrder::Tags => pairs.sort_by(|(_, a), (_, b)| {
                let ta = a
                    .tags
                    .as_deref()
                    .and_then(|s| s.first())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let tb = b
                    .tags
                    .as_deref()
                    .and_then(|s| s.first())
                    .map(|s| s.as_str())
                    .unwrap_or("");
                ta.cmp(tb)
            }),
        }
        let filtered_tasks: Vec<&Task> = pairs.into_iter().map(|(_, t)| t).collect();

        let selected_set = app.task_manager.selected_tasks.clone();
        task_table(
            ui,
            &filtered_tasks,
            &mut app.task_manager.current,
            &selected_set,
        );

        if app.task_manager.sort_picker_open {
            sort_picker_modal(ui, app);
        } else if app.task_manager.status_picker_open {
            status_picker_modal(ui, app);
        } else {
            match app.app_state.mode {
                Mode::Normal => {
                    app.task_manager.writer.flush();
                    if app.task_manager.filter_open {
                        filter_mode_keybinds(ui, app);
                    } else {
                        normal_mode_keybinds(ui, app, &visible, vis_len);
                    }
                    app.task_manager.writer.has_focus = EditFocus::None;
                }
                Mode::Visual => visual_mode_keybinds(ui, app, &visible, vis_len),
                Mode::Insert(_) => {
                    // Editor is in the Info pane; task_state just shows the list
                }
            }
        }
    })
}

/// Read-only summary card for a task — title, details, and metadata grid.
/// Used in both the bottom detail panel and the Info pane.
pub(crate) fn task_card(ui: &mut egui::Ui, task: &Task) {
    use crate::ui::theme::colors;
    use crate::ui::widgets::common;
    ui.label(egui::RichText::new(&task.title).size(16.0).strong());
    ui.separator();
    if !task.details.is_empty() {
        egui::ScrollArea::vertical()
            .id_salt("task_card_details")
            .show(ui, |ui| {
                ui.label(egui::RichText::new(&task.details).size(13.0));
            });
        ui.add_space(4.0);
    }
    egui::Grid::new("task_card_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            common::field_label(ui, "Status");
            common::status_badge(ui, &task.status);
            ui.end_row();

            common::field_label(ui, "Priority");
            ui.label(
                egui::RichText::new(task.priority.to_string())
                    .color(crate::ui::theme::priority_color(&task.priority)),
            );
            ui.end_row();

            if let Some(due) = task.due {
                common::field_label(ui, "Due");
                ui.label(egui::RichText::new(format_due_short(&due)).color(due_date_color(&due)));
                ui.end_row();
            }

            if let Some(tags) = &task.tags
                && !tags.is_empty()
            {
                common::field_label(ui, "Tags");
                ui.label(tags.join(", "));
                ui.end_row();
            }

            if let Some(wait) = task.wait_until {
                common::field_label(ui, "Hidden until");
                ui.label(egui::RichText::new(format_due_short(&wait)).color(colors::SUBTEXT0));
                ui.end_row();
            }

            if let Some(recurrence) = &task.recurrence {
                common::field_label(ui, "Recurrence");
                ui.label(egui::RichText::new(recurrence.to_string()).color(colors::TEAL));
                ui.end_row();
            }
        });
}

/// Renders the bottom detail pane. Returns `true` if the `✎ Edit` button was clicked
/// (the caller enters Insert mode on the current task).
fn detail_panel(ui: &mut egui::Ui, task: Option<Task>) -> bool {
    let mut edit_clicked = false;
    egui::Frame::new()
        .inner_margin(egui::Margin::same(8_i8))
        .show(ui, |ui| {
            if let Some(task) = task {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::ui::widgets::common::secondary_button(
                            ui,
                            format!("{}  Edit", crate::ui::theme::icons::MODE_INSERT),
                        )
                        .on_hover_text("Edit this task (i / e)")
                        .clicked()
                        {
                            edit_clicked = true;
                        }
                    });
                });
                task_card(ui, &task);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("No task selected")
                            .weak()
                            .italics()
                            .size(11.0),
                    );
                });
            }
        });
    edit_clicked
}

/// Spawns a background thread to fetch tasks for `lookup` and sends the result via `tx`.
pub fn get_tasks(
    backend: Backend,
    lookup: ProjectEntry,
    show_completed: bool,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || {
        match backend
            .get_tasks(lookup)
            .context("No tasks found for the project")
        {
            Ok(result) => {
                let visible: Vec<Task> = result
                    .into_iter()
                    .filter(|t| show_completed || t.status != TaskStatus::Completed)
                    .collect();
                tx.send(UpdateMessage::Tasks(visible))
            }
            Err(e) => tx.send(UpdateMessage::Error(e)),
        }
    });
}

/// Spawns a background thread to apply `writer` edits to an existing task.
pub(crate) fn task_submit_edit(
    backend: Backend,
    writer: TaskWriter,
    task_id: ObjectId,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || match backend.one_task(task_id) {
        Ok(Some(mut task)) => {
            task.title = writer.title_buffer.clone();
            task.details = writer.details_buffer.clone();
            task.code = writer.code;
            task.status = writer.status;
            task.due = writer.duedate;
            task.wait_until = writer.wait_until;
            task.priority = writer.priority;
            task.tags = parse_tags(&writer.tags_buffer);
            task.recurrence = writer.recurrence;
            let _ = match backend.update_task(task) {
                Ok(result) => tx.send(UpdateMessage::DbTransaction(Box::new(result))),
                Err(e) => tx.send(UpdateMessage::Error(e)),
            };
        }
        Ok(None) => {
            let _ = tx.send(UpdateMessage::Error(anyhow::anyhow!("Task not found")));
        }
        Err(e) => {
            let _ = tx.send(UpdateMessage::Error(e));
        }
    });
}

/// Spawns a background thread to create a new task from `writer` under `project`.
pub(crate) fn task_submit_create(
    backend: Backend,
    writer: TaskWriter,
    project: ProjectEntry,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || {
        let tags = parse_tags(&writer.tags_buffer);
        let task = Task {
            title: writer.title_buffer.clone(),
            details: writer.details_buffer.clone(),
            project_id: project.get_id(),
            priority: writer.priority,
            due: writer.duedate,
            tags,
            order: writer.order,
            code: writer.code,
            status: writer.status,
            wait_until: writer.wait_until,
            recurrence: writer.recurrence,
            ..Default::default()
        };
        match backend.create_task(task) {
            Ok(result) => {
                tx.send(UpdateMessage::DbTransaction(Box::new(result))).ok();
            }
            Err(e) => {
                tx.send(UpdateMessage::Error(e)).ok();
            }
        }
    });
}

/// Paste a yanked task as a new task, resetting id, order, status, and modify_date.
fn task_submit_paste(
    backend: Backend,
    source: Task,
    project_id: Option<ObjectId>,
    order: u64,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || {
        let task = Task {
            title: source.title,
            details: source.details,
            priority: source.priority,
            due: source.due,
            tags: source.tags,
            code: source.code,
            wait_until: source.wait_until,
            project_id,
            order,
            ..Default::default()
        };
        match backend.create_task(task) {
            Ok(result) => {
                tx.send(UpdateMessage::DbTransaction(Box::new(result))).ok();
            }
            Err(e) => {
                tx.send(UpdateMessage::Error(e)).ok();
            }
        }
    });
}

fn task_submit_complete(
    backend: Backend,
    task_id: ObjectId,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    task_submit_set_status(backend, task_id, TaskStatus::Completed, tx);
}

fn task_submit_complete_many(
    backend: Backend,
    ids: Vec<ObjectId>,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    task_submit_set_status_many(backend, ids, TaskStatus::Completed, tx);
}

fn task_submit_delete(
    backend: Backend,
    task_id: ObjectId,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || match backend.delete_task(task_id) {
        Ok(result) => {
            tx.send(UpdateMessage::DbTransaction(Box::new(result))).ok();
        }
        Err(e) => {
            tx.send(UpdateMessage::Error(e)).ok();
        }
    });
}

fn task_submit_delete_many(
    backend: Backend,
    ids: Vec<ObjectId>,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || {
        for id in ids {
            if let Err(e) = backend.delete_task(id) {
                let _ = tx.send(UpdateMessage::Error(e));
                return;
            }
        }
        tx.send(UpdateMessage::Refresh).ok();
    });
}

fn task_submit_set_status(
    backend: Backend,
    task_id: ObjectId,
    status: TaskStatus,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || match backend.one_task(task_id) {
        Ok(Some(mut task)) => {
            if status == TaskStatus::Completed
                && let Some(recurrence) = &task.recurrence
            {
                let base = task
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
                if let Ok(next_date) = base.checked_add(span) {
                    let next_task = Task {
                        title: task.title.clone(),
                        details: task.details.clone(),
                        priority: task.priority.clone(),
                        tags: task.tags.clone(),
                        code: task.code,
                        project_id: task.project_id,
                        recurrence: task.recurrence.clone(),
                        wait_until: task.wait_until,
                        due: from_jiff_to_datetime(next_date),
                        order: task.get_next_gap(),
                        ..Default::default()
                    };
                    if let Err(e) = backend.create_task(next_task) {
                        let _ = tx.send(UpdateMessage::Error(e));
                        return;
                    }
                }
            }
            task.status = status;
            match backend.update_task(task) {
                Ok(result) => {
                    tx.send(UpdateMessage::DbTransaction(Box::new(result))).ok();
                }
                Err(e) => {
                    tx.send(UpdateMessage::Error(e)).ok();
                }
            }
        }
        Ok(None) => {
            tx.send(UpdateMessage::Error(anyhow::anyhow!("Task not found")))
                .ok();
        }
        Err(e) => {
            tx.send(UpdateMessage::Error(e)).ok();
        }
    });
}

fn task_submit_set_status_many(
    backend: Backend,
    ids: Vec<ObjectId>,
    status: TaskStatus,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || {
        for id in ids {
            match backend.one_task(id) {
                Ok(Some(mut task)) => {
                    task.status = status.clone();
                    if let Err(e) = backend.update_task(task) {
                        let _ = tx.send(UpdateMessage::Error(e));
                        return;
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = tx.send(UpdateMessage::Error(e));
                    return;
                }
            }
        }
        let _ = tx.send(UpdateMessage::Refresh);
    });
}

fn status_picker_modal(ui: &mut egui::Ui, app: &mut FastTask) {
    use crate::ui::theme::colors;

    const CHOICES: [(TaskStatus, &str); 4] = [
        (TaskStatus::NotStarted, "Not Started"),
        (TaskStatus::InProgress, "In Progress"),
        (TaskStatus::OnHold, "On Hold"),
        (TaskStatus::Completed, "Completed"),
    ];

    let (nav_j, nav_k, confirmed, dismissed, num_choice) = ui.input(|i| {
        let j = i.key_pressed(egui::Key::J) || i.key_pressed(egui::Key::ArrowDown);
        let k = i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::ArrowUp);
        let enter = i.key_pressed(egui::Key::Enter);
        let esc = i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::S);
        let num = if i.key_pressed(egui::Key::Num1) {
            Some(0usize)
        } else if i.key_pressed(egui::Key::Num2) {
            Some(1)
        } else if i.key_pressed(egui::Key::Num3) {
            Some(2)
        } else if i.key_pressed(egui::Key::Num4) {
            Some(3)
        } else {
            None
        };
        (j, k, enter, esc, num)
    });

    let close = |app: &mut FastTask| {
        app.task_manager.status_picker_open = false;
        app.task_manager.status_picker_task_id = None;
        app.task_manager.status_picker_ids.clear();
    };

    if dismissed {
        close(app);
        return;
    }

    if nav_j {
        app.task_manager.status_picker_cursor =
            (app.task_manager.status_picker_cursor + 1).min(CHOICES.len() - 1);
    }
    if nav_k {
        app.task_manager.status_picker_cursor =
            app.task_manager.status_picker_cursor.saturating_sub(1);
    }

    let mut chosen: Option<TaskStatus> = num_choice.map(|i| CHOICES[i].0.clone());
    if confirmed {
        chosen = Some(CHOICES[app.task_manager.status_picker_cursor].0.clone());
    }

    if let Some(status) = chosen {
        let tx = app.backend_manager.tx.clone();
        let backend = app.backend_manager.backend.clone();
        if !app.task_manager.status_picker_ids.is_empty() {
            let ids = app.task_manager.status_picker_ids.clone();
            app.task_manager.selected_tasks.clear();
            task_submit_set_status_many(backend, ids, status, tx);
        } else if let Some(task_id) = app.task_manager.status_picker_task_id {
            task_submit_set_status(backend, task_id, status, tx);
        }
        close(app);
        return;
    }

    let cursor = app.task_manager.status_picker_cursor;
    let multi_count = app.task_manager.status_picker_ids.len();
    egui::Modal::new(egui::Id::new("status_picker")).show(ui.ctx(), |ui| {
        ui.set_min_width(200.0);
        let title = if multi_count > 1 {
            format!("Set Status  ({} tasks)", multi_count)
        } else {
            "Set Status".to_string()
        };
        ui.label(
            egui::RichText::new(title)
                .color(colors::LAVENDER)
                .size(14.0)
                .strong(),
        );
        ui.separator();
        ui.add_space(4.0);

        for (idx, (status, label)) in CHOICES.iter().enumerate() {
            let is_selected = idx == cursor;
            let color = if is_selected {
                colors::MANTLE
            } else {
                crate::ui::theme::status_color(status)
            };
            let text = egui::RichText::new(format!("{}  {}", idx + 1, label))
                .color(color)
                .size(13.0);
            if ui.selectable_label(is_selected, text).clicked() {
                let tx = app.backend_manager.tx.clone();
                let backend = app.backend_manager.backend.clone();
                if multi_count > 1 {
                    let ids = app.task_manager.status_picker_ids.clone();
                    app.task_manager.selected_tasks.clear();
                    task_submit_set_status_many(backend, ids, status.clone(), tx);
                } else if let Some(task_id) = app.task_manager.status_picker_task_id {
                    task_submit_set_status(backend, task_id, status.clone(), tx);
                }
                app.task_manager.status_picker_open = false;
                app.task_manager.status_picker_task_id = None;
                app.task_manager.status_picker_ids.clear();
            }
        }

        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("j/k  ·  Enter  ·  1-4  ·  Esc")
                .color(colors::OVERLAY0)
                .size(11.0),
        );
    });
}

fn sort_picker_modal(ui: &mut egui::Ui, app: &mut FastTask) {
    use crate::ui::theme::colors;

    const CHOICES: [(SortOrder, &str); 5] = [
        (SortOrder::Free, "Free  (manual order)"),
        (SortOrder::DueDate, "Due Date"),
        (SortOrder::Modified, "Modified"),
        (SortOrder::Status, "Status"),
        (SortOrder::Tags, "Tags"),
    ];

    let (nav_j, nav_k, confirmed, dismissed, num_choice) = ui.input(|i| {
        let j = i.key_pressed(egui::Key::J) || i.key_pressed(egui::Key::ArrowDown);
        let k = i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::ArrowUp);
        let enter = i.key_pressed(egui::Key::Enter);
        let esc = i.key_pressed(egui::Key::Escape);
        let num = if i.key_pressed(egui::Key::Num1) {
            Some(0usize)
        } else if i.key_pressed(egui::Key::Num2) {
            Some(1)
        } else if i.key_pressed(egui::Key::Num3) {
            Some(2)
        } else if i.key_pressed(egui::Key::Num4) {
            Some(3)
        } else if i.key_pressed(egui::Key::Num5) {
            Some(4)
        } else {
            None
        };
        (j, k, enter, esc, num)
    });

    if dismissed {
        app.task_manager.sort_picker_open = false;
        return;
    }
    if nav_j {
        app.task_manager.sort_picker_cursor =
            (app.task_manager.sort_picker_cursor + 1).min(CHOICES.len() - 1);
    }
    if nav_k {
        app.task_manager.sort_picker_cursor = app.task_manager.sort_picker_cursor.saturating_sub(1);
    }

    let mut chosen: Option<SortOrder> = num_choice.map(|i| CHOICES[i].0.clone());
    if confirmed {
        chosen = Some(CHOICES[app.task_manager.sort_picker_cursor].0.clone());
    }

    if let Some(order) = chosen {
        app.task_manager.sort_order = order;
        app.task_manager.sort_picker_open = false;
        return;
    }

    let cursor = app.task_manager.sort_picker_cursor;
    egui::Modal::new(egui::Id::new("sort_picker")).show(ui.ctx(), |ui| {
        ui.set_min_width(220.0);
        ui.label(
            egui::RichText::new("Sort Tasks")
                .color(colors::LAVENDER)
                .size(14.0)
                .strong(),
        );
        ui.separator();
        ui.add_space(4.0);

        for (idx, (order, label)) in CHOICES.iter().enumerate() {
            let is_selected = idx == cursor;
            let is_active = *order == app.task_manager.sort_order;
            let color = if is_selected {
                colors::MANTLE
            } else if is_active {
                colors::TEAL
            } else {
                colors::TEXT
            };
            let prefix = if is_active { "✓ " } else { "  " };
            let text = egui::RichText::new(format!("{}{}  {}", prefix, idx + 1, label))
                .color(color)
                .size(13.0);
            let fill = if is_selected {
                colors::BLUE
            } else {
                egui::Color32::TRANSPARENT
            };
            let btn = egui::Button::new(text)
                .fill(fill)
                .stroke(egui::Stroke::NONE);
            if ui.add(btn).clicked() {
                app.task_manager.sort_order = order.clone();
                app.task_manager.sort_picker_open = false;
            }
        }

        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("j/k  ·  Enter  ·  1-5  ·  Esc")
                .color(colors::OVERLAY0)
                .size(11.0),
        );
    });
}

// --- Order helpers ---

/// Returns an order value that places a new task immediately after `tasks[idx]`.
pub(crate) fn order_below(tasks: &[Task], idx: usize) -> u64 {
    let current = tasks[idx].order;
    if idx + 1 < tasks.len() {
        let next = tasks[idx + 1].order;
        if next > current + 1 {
            (current + next) / 2
        } else {
            tasks
                .last()
                .map(|t| t.order + ORDER_GAP)
                .unwrap_or(ORDER_GAP)
        }
    } else {
        current + ORDER_GAP
    }
}

/// Returns an order value that places a new task immediately before `tasks[idx]`.
pub(crate) fn order_above(tasks: &[Task], idx: usize) -> u64 {
    let current = tasks[idx].order;
    if idx > 0 {
        let prev = tasks[idx - 1].order;
        if current > prev + 1 {
            (prev + current) / 2
        } else {
            tasks
                .last()
                .map(|t| t.order + ORDER_GAP)
                .unwrap_or(ORDER_GAP)
        }
    } else if current > 1 {
        current / 2
    } else {
        ORDER_GAP
    }
}

fn normal_mode_keybinds(ui: &egui::Ui, app: &mut FastTask, visible: &[usize], vis_len: usize) {
    ui.input(|i| {
        let shift = i.modifiers.shift;

        // Escape clears Tab-selection if any (mode is already Normal)
        if i.key_pressed(Key::Escape) && !app.task_manager.selected_tasks.is_empty() {
            app.task_manager.selected_tasks.clear();
        }

        // v = Visual mode (single-cursor reorder)
        if i.key_pressed(Key::V) && !shift {
            app.app_state.mode = Mode::Visual;
        }

        // Shift+S = sort picker
        if i.key_pressed(Key::S) && shift {
            app.task_manager.sort_picker_open = true;
            app.task_manager.sort_picker_cursor = match app.task_manager.sort_order {
                SortOrder::Free => 0,
                SortOrder::DueDate => 1,
                SortOrder::Modified => 2,
                SortOrder::Status => 3,
                SortOrder::Tags => 4,
            };
        }

        // Tab toggles the current task in/out of the selection set (cursor does not move)
        let current_task_tab = app
            .task_manager
            .current
            .and_then(|i| visible.get(i).copied())
            .and_then(|ri| app.task_manager.tasks.get(ri))
            .cloned();
        if i.key_pressed(Key::Tab)
            && let Some(task) = current_task_tab
            && !app.task_manager.selected_tasks.remove(&task.id)
        {
            app.task_manager.selected_tasks.insert(task.id);
        }

        // Shift+K toggles the detail pane
        if shift && i.key_pressed(Key::K) {
            app.app_state.show_detail_pane = !app.app_state.show_detail_pane;
        }

        // Shift+C toggles completed tasks in the list
        if shift && i.key_pressed(Key::C) {
            app.task_manager.show_completed = !app.task_manager.show_completed;
            let show = app.task_manager.show_completed;
            get_tasks(
                app.backend_manager.backend.clone(),
                app.project_manager.current().clone(),
                show,
                app.backend_manager.tx.clone(),
            );
        }

        // / = open filter bar
        if i.key_pressed(Key::Slash) {
            app.task_manager.filter_open = true;
            app.task_manager.filter_just_opened = true;
        }

        // o = insert below, O = insert above; uses real index for correct ordering
        if i.key_pressed(Key::O) {
            if let Some(vis_idx) = app.task_manager.current {
                let real_idx = visible
                    .get(vis_idx)
                    .copied()
                    .unwrap_or(0)
                    .min(app.task_manager.tasks.len().saturating_sub(1));
                app.task_manager.writer.order = if app.task_manager.tasks.is_empty() {
                    ORDER_GAP
                } else if shift {
                    order_above(&app.task_manager.tasks, real_idx)
                } else {
                    order_below(&app.task_manager.tasks, real_idx)
                };
            } else {
                app.task_manager.writer.order = app
                    .task_manager
                    .tasks
                    .last()
                    .map(|t| t.order + ORDER_GAP)
                    .unwrap_or(ORDER_GAP);
            }
            app.app_state.mode = Mode::Insert(None);
            app.app_state.window_state = WindowState::Info;
        }

        // y = yank (copy) current task into clipboard
        if i.key_pressed(Key::Y)
            && let Some(task) = app
                .task_manager
                .current
                .and_then(|i| visible.get(i).copied())
                .and_then(|ri| app.task_manager.tasks.get(ri))
                .cloned()
        {
            app.task_manager.clipboard_task = Some(task);
        }

        // p = paste clipboard below cursor, or navigate to Projects if clipboard is empty
        if i.key_pressed(Key::P) {
            if let Some(source) = app.task_manager.clipboard_task.clone() {
                let order = if let Some(vis_idx) = app.task_manager.current {
                    let real_idx = visible.get(vis_idx).copied().unwrap_or(0);
                    order_below(&app.task_manager.tasks, real_idx)
                } else {
                    app.task_manager
                        .tasks
                        .last()
                        .map(|t| t.order + ORDER_GAP)
                        .unwrap_or(ORDER_GAP)
                };
                task_submit_paste(
                    app.backend_manager.backend.clone(),
                    source,
                    app.project_manager.current().get_id(),
                    order,
                    app.backend_manager.tx.clone(),
                );
            } else {
                app.app_state.window_state = WindowState::Projects;
            }
        }

        if i.key_pressed(Key::J) {
            app.task_manager.current = Some(match app.task_manager.current {
                Some(i) => (i + 1).min(vis_len.saturating_sub(1)),
                None => 0,
            });
        }

        if i.key_pressed(Key::K) && !shift {
            app.task_manager.current = Some(match app.task_manager.current {
                Some(i) => i.saturating_sub(1),
                None => 0,
            });
        }

        let current_task = app
            .task_manager
            .current
            .and_then(|i| visible.get(i).copied())
            .and_then(|ri| app.task_manager.tasks.get(ri))
            .cloned();
        if let Some(current_task) = current_task {
            // d = mark complete, Shift+D = hard delete
            // If there's a Tab-selection set, operate on all; otherwise on current task
            if i.key_pressed(Key::D) {
                let backend = app.backend_manager.backend.clone();
                let tx = app.backend_manager.tx.clone();
                if !app.task_manager.selected_tasks.is_empty() {
                    let ids: Vec<ObjectId> =
                        app.task_manager.selected_tasks.iter().copied().collect();
                    if shift {
                        task_submit_delete_many(backend, ids, tx);
                    } else {
                        task_submit_complete_many(backend, ids, tx);
                    }
                    app.task_manager.selected_tasks.clear();
                } else if let Some(idx) = app.task_manager.current {
                    if shift {
                        task_submit_delete(backend, current_task.id, tx);
                        app.app_state.status_msg =
                            Some(("Deleted — u to undo".to_string(), std::time::Instant::now()));
                    } else {
                        task_submit_complete(backend, current_task.id, tx);
                    }
                    app.task_manager.current = idx.checked_sub(1);
                }
            }

            // s = set status; opens picker for single task or the whole selection set
            if i.key_pressed(Key::S) {
                app.task_manager.status_picker_cursor = 0;
                app.task_manager.status_picker_open = true;
                if !app.task_manager.selected_tasks.is_empty() {
                    app.task_manager.status_picker_ids =
                        app.task_manager.selected_tasks.iter().copied().collect();
                    app.task_manager.status_picker_task_id = None;
                } else {
                    app.task_manager.status_picker_ids.clear();
                    app.task_manager.status_picker_task_id = Some(current_task.id);
                }
            }

            // e / i = edit in Info pane
            if i.key_pressed(Key::E) || i.key_pressed(Key::I) {
                app.app_state.mode = Mode::Insert(Some(current_task.id));
                app.app_state.window_state = WindowState::Info;
            }
        }
    });
}

fn filter_mode_keybinds(ui: &egui::Ui, app: &mut FastTask) {
    ui.input(|i| {
        if i.key_pressed(Key::Escape) {
            app.task_manager.filter_query.clear();
            app.task_manager.filter_open = false;
        }
        if i.key_pressed(Key::Enter) {
            app.task_manager.filter_open = false;
        }
    });
    // Keep the filter TextEdit focused while the bar is open so any key routes there.
    let filter_id = egui::Id::new("task_filter_input");
    if !ui.ctx().memory(|m| m.has_focus(filter_id)) {
        ui.ctx().memory_mut(|m| m.request_focus(filter_id));
    }
}

/// Visual mode (`v`): single-cursor reordering. j/k navigate, Shift+J/K swap order.
fn visual_mode_keybinds(ui: &egui::Ui, app: &mut FastTask, visible: &[usize], vis_len: usize) {
    ui.input(|i| {
        if i.key_pressed(Key::Escape) {
            app.app_state.mode = Mode::Normal;
        }

        let shift = i.modifiers.shift;

        if i.key_pressed(Key::J) && !shift {
            app.task_manager.current = Some(match app.task_manager.current {
                Some(i) => (i + 1).min(vis_len.saturating_sub(1)),
                None => 0,
            });
        }
        if i.key_pressed(Key::K) && !shift {
            app.task_manager.current = Some(match app.task_manager.current {
                Some(i) => i.saturating_sub(1),
                None => 0,
            });
        }

        // Shift+J / Shift+K: move cursor task down / up (only in Free sort, no active filter)
        if app.task_manager.sort_order == SortOrder::Free
            && app.task_manager.filter_query.is_empty()
            && let Some(a) = app.task_manager.current
        {
            if shift && i.key_pressed(Key::J) && a + 1 < app.task_manager.tasks.len() {
                swap_tasks(app, app.backend_manager.backend.clone(), a, a + 1);
                app.task_manager.current = Some(a + 1);
            } else if shift && i.key_pressed(Key::K) && a > 0 {
                swap_tasks(app, app.backend_manager.backend.clone(), a, a - 1);
                app.task_manager.current = Some(a - 1);
            }
        }

        let current_task = app
            .task_manager
            .current
            .and_then(|i| visible.get(i).copied())
            .and_then(|ri| app.task_manager.tasks.get(ri))
            .cloned();
        if let Some(current_task) = current_task {
            if i.key_pressed(Key::D) {
                let backend = app.backend_manager.backend.clone();
                let tx = app.backend_manager.tx.clone();
                if shift {
                    task_submit_delete(backend, current_task.id, tx);
                } else {
                    task_submit_complete(backend, current_task.id, tx);
                }
                app.app_state.mode = Mode::Normal;
            }
            if i.key_pressed(Key::S) {
                app.task_manager.status_picker_open = true;
                app.task_manager.status_picker_task_id = Some(current_task.id);
                app.task_manager.status_picker_cursor = 0;
            }
            if i.key_pressed(Key::E) || i.key_pressed(Key::I) {
                app.app_state.mode = Mode::Insert(Some(current_task.id));
                app.app_state.window_state = WindowState::Info;
            }
        }
    });
}

fn swap_tasks(app: &mut FastTask, backend: Backend, a: usize, b: usize) {
    if b < app.task_manager.tasks.len() {
        let a_o = app.task_manager.tasks[a].order;
        let b_o = app.task_manager.tasks[b].order;
        app.task_manager.tasks[a].order = b_o;
        app.task_manager.tasks[b].order = a_o;
        let ta = app.task_manager.tasks[a].clone();
        let tb = app.task_manager.tasks[b].clone();
        app.task_manager.tasks.swap(a, b);
        let tx = app.backend_manager.tx.clone();
        std::thread::spawn(move || {
            for task in [ta, tb] {
                match backend.update_task(task) {
                    Ok(r) => {
                        tx.send(UpdateMessage::DbTransaction(Box::new(r))).ok();
                    }
                    Err(e) => {
                        tx.send(UpdateMessage::Error(e)).ok();
                        return;
                    }
                }
            }
        });
    }
}

/// Converts a BSON `DateTime` to a `jiff` civil date in UTC.
pub(crate) fn bson_dt_to_jiff_date(dt: &DateTime) -> Option<jiff::civil::Date> {
    let ts = jiff::Timestamp::from_millisecond(dt.timestamp_millis()).ok()?;
    Some(ts.to_zoned(jiff::tz::TimeZone::UTC).date())
}

/// Formats a due date as "Mon D" (current year) or "Mon D, YYYY" (other years).
pub(crate) fn format_due_short(dt: &DateTime) -> String {
    let Some(date) = bson_dt_to_jiff_date(dt) else {
        return String::new();
    };
    let today = jiff::Zoned::now().date();
    let month = match date.month() {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    };
    if date.year() == today.year() {
        format!("{} {}", month, date.day())
    } else {
        format!("{} {}, {}", month, date.day(), date.year())
    }
}

/// Returns RED for overdue, YELLOW for today, SUBTEXT0 for future dates.
pub(crate) fn due_date_color(dt: &DateTime) -> egui::Color32 {
    use crate::ui::theme::colors;
    let Some(date) = bson_dt_to_jiff_date(dt) else {
        return colors::SUBTEXT0;
    };
    let today = jiff::Zoned::now().date();
    if date < today {
        colors::RED
    } else if date == today {
        colors::YELLOW
    } else {
        colors::SUBTEXT0
    }
}

fn task_table(
    ui: &mut egui::Ui,
    tasks: &[&Task],
    selected: &mut Option<usize>,
    tab_selected: &std::collections::HashSet<ObjectId>,
) {
    use crate::ui::theme::colors;

    if tasks.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("No tasks here.\no / O  — new task below / above")
                    .color(colors::OVERLAY0)
                    .size(11.0),
            );
        });
        return;
    }

    egui_extras::TableBuilder::new(ui)
        .striped(true)
        .resizable(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(egui_extras::Column::remainder())
        .min_scrolled_height(0.0)
        .body(|body| {
            body.rows(26.0, tasks.len(), |mut row| {
                let row_index = row.index();
                let is_cursor = *selected == Some(row_index);
                let task = &tasks[row_index];
                let is_tab_sel = tab_selected.contains(&task.id);

                row.col(|ui| {
                    let rect = ui.max_rect();

                    let response = ui.interact(rect, ui.id().with(row_index), Sense::click());
                    if response.clicked() {
                        *selected = Some(row_index);
                    }

                    if is_cursor {
                        ui.painter().rect_filled(rect, 3.0, colors::BLUE);
                    } else if is_tab_sel {
                        ui.painter().rect_filled(rect, 3.0, colors::TEAL_DIM);
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 3.0, colors::SURFACE1);
                    }

                    ui.add_space(6.0);

                    let is_highlighted = is_cursor || is_tab_sel;
                    // Title color: MANTLE on highlighted rows; TEXT normally; SUBTEXT0 for Completed.
                    // Status color is expressed only through the status icon, not the title.
                    let text_color = if is_highlighted {
                        colors::MANTLE
                    } else if task.status == crate::database::models::TaskStatus::Completed {
                        colors::SUBTEXT0
                    } else {
                        colors::TEXT
                    };

                    use crate::ui::theme::icons;
                    let status_sym = if is_tab_sel && !is_cursor {
                        "✓"
                    } else {
                        match task.status {
                            crate::database::models::TaskStatus::NotStarted => {
                                icons::STATUS_NOT_STARTED
                            }
                            crate::database::models::TaskStatus::InProgress => {
                                icons::STATUS_IN_PROGRESS
                            }
                            crate::database::models::TaskStatus::OnHold => icons::STATUS_ON_HOLD,
                            crate::database::models::TaskStatus::Completed => {
                                icons::STATUS_COMPLETED
                            }
                        }
                    };
                    let status_color = if is_highlighted {
                        colors::MANTLE
                    } else {
                        crate::ui::theme::status_color(&task.status)
                    };

                    let priority_hint = match task.priority {
                        Priority::Urgent => icons::PRIORITY_URGENT,
                        Priority::Normal => "",
                        Priority::Low => icons::PRIORITY_LOW,
                    };
                    let priority_color = if is_highlighted {
                        colors::MANTLE
                    } else {
                        crate::ui::theme::priority_color(&task.priority)
                    };

                    let status_tip = match task.status {
                        crate::database::models::TaskStatus::NotStarted => {
                            "Not started (s to change)"
                        }
                        crate::database::models::TaskStatus::InProgress => {
                            "In progress (s to change)"
                        }
                        crate::database::models::TaskStatus::OnHold => "On hold (s to change)",
                        crate::database::models::TaskStatus::Completed => "Completed (s to change)",
                    };
                    let priority_tip = match task.priority {
                        Priority::Urgent => "Urgent priority",
                        Priority::Normal => "",
                        Priority::Low => "Low priority",
                    };
                    let render_row = |ui: &mut egui::Ui| {
                        ui.label(egui::RichText::new(status_sym.to_string()).color(status_color))
                            .on_hover_text(status_tip);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("  {}", task.title)).color(text_color),
                            )
                            .truncate(),
                        );
                        if !priority_hint.is_empty() {
                            ui.label(egui::RichText::new(priority_hint).color(priority_color))
                                .on_hover_text(priority_tip);
                        }
                    };

                    if let Some(ref due) = task.due {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(6.0);
                            let due_color = if is_highlighted {
                                colors::MANTLE
                            } else {
                                due_date_color(due)
                            };
                            ui.label(
                                egui::RichText::new(format_due_short(due))
                                    .color(due_color)
                                    .size(11.0),
                            );
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    render_row(ui);
                                },
                            );
                        });
                    } else {
                        render_row(ui);
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_with_order(order: u64) -> Task {
        Task {
            title: "t".to_string(),
            order,
            ..Default::default()
        }
    }

    // --- order_below ---

    #[test]
    fn order_below_last_task_appends() {
        let tasks = vec![task_with_order(1000)];
        assert_eq!(order_below(&tasks, 0), 2000);
    }

    #[test]
    fn order_below_inserts_midpoint() {
        let tasks = vec![task_with_order(1000), task_with_order(3000)];
        assert_eq!(order_below(&tasks, 0), 2000);
    }

    #[test]
    fn order_below_no_gap_falls_back_to_end() {
        // Adjacent orders with no room: falls back to last + ORDER_GAP
        let tasks = vec![task_with_order(1000), task_with_order(1001)];
        assert_eq!(order_below(&tasks, 0), 2001);
    }

    // --- order_above ---

    #[test]
    fn order_above_first_task_halves() {
        let tasks = vec![task_with_order(1000)];
        assert_eq!(order_above(&tasks, 0), 500);
    }

    #[test]
    fn order_above_inserts_midpoint() {
        let tasks = vec![task_with_order(1000), task_with_order(3000)];
        assert_eq!(order_above(&tasks, 1), 2000);
    }

    #[test]
    fn order_above_first_task_with_order_zero_uses_gap() {
        let tasks = vec![task_with_order(0)];
        // current = 0, idx = 0, no prev → current/2 = 0 but that's ≤ 1, so ORDER_GAP
        assert_eq!(order_above(&tasks, 0), ORDER_GAP);
    }

    // --- Crash regression: stale cursor after project switch ---

    #[test]
    fn order_below_clamped_idx_does_not_panic() {
        // Simulates pressing 'o' after switching to a 1-task project
        // when the cursor was at index 4 in the previous project.
        let tasks = vec![task_with_order(1000)];
        let stale_idx = 4_usize;
        // Defensive clamp (as done in normal_mode_keybinds)
        let idx = stale_idx.min(tasks.len().saturating_sub(1));
        assert_eq!(order_below(&tasks, idx), 2000); // doesn't panic
    }

    // --- H7 seam: backend injection ---

    #[test]
    fn get_tasks_works_with_injected_backend() {
        use crate::database::TaskManagement;
        use crate::database::database::Db;
        use crate::ui::app::UpdateMessage;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("test.db")).unwrap();
        let task = Task {
            title: "injected task".to_string(),
            order: 1000,
            ..Default::default()
        };
        db.create_task(task).unwrap();

        let backend: Backend = std::sync::Arc::new(db);
        let (tx, rx) = std::sync::mpsc::channel();
        get_tasks(backend, crate::database::ProjectEntry::All, false, tx);

        let msg = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        match msg {
            UpdateMessage::Tasks(tasks) => assert_eq!(tasks[0].title, "injected task"),
            other => panic!("unexpected message: {:?}", other),
        }
    }
}
