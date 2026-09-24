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
            SortOrder::Free => "Manual",
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
    /// Yanked tasks, in list order; `p` / `Shift+P` paste all of them.
    pub clipboard: Vec<Task>,
    /// First `g` of a `gg` (jump to top) was pressed; cleared by any other key.
    pub pending_g: bool,
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
    /// Highlighted tag completion; `None` = not moved yet (top item is highlighted).
    pub tag_suggestion_idx: Option<usize>,
    /// Text the completion menu was last built for; a change resets the menu.
    pub tag_menu_query: String,
    /// Menu closed with Esc / Ctrl+E; reopens once the text changes.
    pub tag_menu_dismissed: bool,
    /// Filtered indices into `tasks`, recomputed once per frame via
    /// `refresh_visible_cache`. Read by `real_index`/`get_current_task` so they
    /// don't rebuild the list (and re-syscall the clock) on every call.
    pub visible_cache: Vec<usize>,
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

    /// Recompute `visible_cache`. Call once per frame, after task refreshes and
    /// the status-bar filter input have settled, before any pane reads the
    /// cursor via `real_index`/`get_current_task`.
    pub fn refresh_visible_cache(&mut self) {
        self.visible_cache = self.visible_indices();
    }

    /// Translate a visible-list cursor position to a real `tasks[]` index.
    /// Reads the per-frame `visible_cache` (see `refresh_visible_cache`).
    pub fn real_index(&self, visible: usize) -> Option<usize> {
        self.visible_cache.get(visible).copied()
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

    // Reuse the per-frame cache (refreshed before the window_state dispatch);
    // shared by table, keybinds, and cursor clamp.
    let visible = app.task_manager.visible_cache.clone();
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
                .default_size(90.0)
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
        let row_action = task_table(
            ui,
            &filtered_tasks,
            &mut app.task_manager.current,
            &selected_set,
        );

        // Dispatch a mouse row action before the picker/mode handling below, so a
        // Status click opens the picker this frame and an Edit click flips to
        // Insert before the mode match reads it (landing on the no-op Insert arm).
        if let Some(act) = row_action {
            match act {
                RowAction::Edit(id) => {
                    app.app_state.mode = Mode::Insert(Some(id));
                    app.app_state.window_state = WindowState::Info;
                }
                RowAction::Complete(id) => {
                    let backend = app.backend_manager.backend.clone();
                    let tx = app.backend_manager.tx.clone();
                    task_submit_complete(backend, id, tx);
                }
                RowAction::Delete(id) => {
                    let backend = app.backend_manager.backend.clone();
                    let tx = app.backend_manager.tx.clone();
                    task_submit_delete(backend, id, tx);
                    app.app_state.status_msg =
                        Some(("Deleted — u to undo".to_string(), std::time::Instant::now()));
                }
                RowAction::Status(id) => {
                    app.task_manager.status_picker_cursor = 0;
                    app.task_manager.status_picker_open = true;
                    app.task_manager.status_picker_ids.clear();
                    app.task_manager.status_picker_task_id = Some(id);
                }
            }
        }

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
/// Used in the Info pane; the bottom detail pane uses the compact `task_summary`.
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

/// Compact at-a-glance summary for the bottom detail pane: title plus one
/// wrapped row of status / priority / due / tags. The details body and notes
/// are left to the Info pane's full `task_card`.
fn task_summary(ui: &mut egui::Ui, task: &Task) {
    use crate::ui::theme::colors;
    use crate::ui::widgets::common;
    ui.label(egui::RichText::new(&task.title).size(14.0).strong());
    ui.horizontal_wrapped(|ui| {
        common::status_badge(ui, &task.status);
        ui.label(
            egui::RichText::new(task.priority.to_string())
                .color(crate::ui::theme::priority_color(&task.priority)),
        )
        .on_hover_text("Priority");
        if let Some(due) = task.due {
            ui.label(egui::RichText::new(format_due_short(&due)).color(due_date_color(&due)))
                .on_hover_text("Due");
        }
        if let Some(tags) = &task.tags {
            for tag in tags {
                ui.label(egui::RichText::new(format!("#{tag}")).color(colors::SUBTEXT0));
            }
        }
        if !task.details.is_empty() {
            ui.label(egui::RichText::new("…").color(colors::OVERLAY1))
                .on_hover_text("Has details — open the task (i / e) to read them");
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
                task_summary(ui, &task);
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
    crate::ui::bg::spawn(move || {
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
    crate::ui::bg::spawn(move || {
        // Only the editor's fields are written; anything else (e.g. `order`, if the
        // task was reordered while the editor was open) is kept from the fresh read.
        let mut just_completed: Option<Task> = None;
        let result = backend.modify_task(task_id, &mut |task| {
            if writer.status == TaskStatus::Completed
                && task.status != TaskStatus::Completed
                && writer.recurrence.is_some()
            {
                just_completed = Some(task.clone());
            }
            task.title = writer.title_buffer.clone();
            task.details = writer.details_buffer.clone();
            task.code = writer.code;
            task.status = writer.status.clone();
            task.due = writer.duedate;
            task.wait_until = writer.wait_until;
            task.priority = writer.priority.clone();
            task.tags = parse_tags(&writer.tags_buffer);
            task.recurrence = writer.recurrence.clone();
            if let Some(done) = &mut just_completed {
                // Base the next occurrence on the saved values, not the old ones.
                *done = task.clone();
            }
        });
        // Save & Done (or picking Completed) on a recurring task schedules the next one.
        if let Some(done) = just_completed
            && let Some(next) = next_occurrence(&done)
            && let Err(e) = backend.create_task(next)
        {
            let _ = tx.send(UpdateMessage::Error(e));
        }
        let _ = match result {
            Ok(Some(id)) => tx.send(UpdateMessage::DbTransaction(Box::new(id))),
            Ok(None) => tx.send(UpdateMessage::Error(anyhow::anyhow!("Task not found"))),
            Err(e) => tx.send(UpdateMessage::Error(e)),
        };
    });
}

/// Spawns a background thread to create a new task from `writer` under `project`.
pub(crate) fn task_submit_create(
    backend: Backend,
    writer: TaskWriter,
    project: ProjectEntry,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    crate::ui::bg::spawn(move || {
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
    crate::ui::bg::spawn(move || {
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
    crate::ui::bg::spawn(move || match backend.delete_task(task_id) {
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
    crate::ui::bg::spawn(move || {
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
    crate::ui::bg::spawn(move || match apply_status(&backend, task_id, &status) {
        Ok(Some(result)) => {
            tx.send(UpdateMessage::DbTransaction(Box::new(result))).ok();
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

/// Set one task's status as an atomic read-modify-write. Completing a recurring
/// task (that wasn't already completed) also creates its next occurrence.
/// Shared by the single and bulk paths so both handle recurrence.
fn apply_status(
    backend: &Backend,
    task_id: ObjectId,
    status: &TaskStatus,
) -> anyhow::Result<Option<ObjectId>> {
    let mut just_completed: Option<Task> = None;
    let result = backend.modify_task(task_id, &mut |task| {
        if *status == TaskStatus::Completed
            && task.status != TaskStatus::Completed
            && task.recurrence.is_some()
        {
            just_completed = Some(task.clone());
        }
        task.status = status.clone();
    })?;
    if let Some(task) = just_completed
        && let Some(next) = next_occurrence(&task)
    {
        backend.create_task(next)?;
    }
    Ok(result)
}

/// The next instance of a recurring `task`, due one period after its due date
/// (or today, if it has none).
fn next_occurrence(task: &Task) -> Option<Task> {
    let recurrence = task.recurrence.as_ref()?;
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
    let next_date = base.checked_add(span).ok()?;
    Some(Task {
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
    })
}

fn task_submit_set_status_many(
    backend: Backend,
    ids: Vec<ObjectId>,
    status: TaskStatus,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    crate::ui::bg::spawn(move || {
        for id in ids {
            if let Err(e) = apply_status(&backend, id, &status) {
                let _ = tx.send(UpdateMessage::Error(e));
                return;
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

/// Half a page for Ctrl+D / Ctrl+U.
const HALF_PAGE: usize = 10;

/// Cursor movement shared by Normal and Visual mode: j/k and ↓/↑ step, `gg` / `G`
/// jump to top / bottom, Ctrl+D / Ctrl+U move half a page. `shift_jk` lets plain
/// j/k also fire with Shift held (Normal); Visual reserves Shift+J/K for reordering.
fn move_cursor(i: &egui::InputState, app: &mut FastTask, vis_len: usize, shift_jk: bool) {
    let shift = i.modifiers.shift;
    let ctrl = i.modifiers.ctrl;
    let last = vis_len.saturating_sub(1);
    let cur = app.task_manager.current;
    let step = |delta: isize| -> Option<usize> {
        Some(match cur {
            Some(c) => c.saturating_add_signed(delta).min(last),
            None => 0,
        })
    };

    let jk_ok = shift_jk || !shift;
    if (i.key_pressed(Key::J) && jk_ok) || i.key_pressed(Key::ArrowDown) {
        app.task_manager.current = step(1);
    }
    if (i.key_pressed(Key::K) && jk_ok && !shift) || i.key_pressed(Key::ArrowUp) {
        app.task_manager.current = step(-1);
    }
    if ctrl && i.key_pressed(Key::D) {
        app.task_manager.current = step(HALF_PAGE as isize);
    }
    if ctrl && i.key_pressed(Key::U) {
        app.task_manager.current = step(-(HALF_PAGE as isize));
    }

    // gg / G. Any other key press cancels a pending first `g`.
    if i.key_pressed(Key::G) {
        if shift {
            app.task_manager.current = (vis_len > 0).then_some(last);
            app.task_manager.pending_g = false;
        } else if app.task_manager.pending_g {
            app.task_manager.current = (vis_len > 0).then_some(0);
            app.task_manager.pending_g = false;
        } else {
            app.task_manager.pending_g = true;
        }
    } else if i
        .events
        .iter()
        .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
    {
        app.task_manager.pending_g = false;
    }
}

/// Tasks the next bulk action applies to: the Tab-selection in list order, or
/// just the task under the cursor when nothing is selected.
fn action_targets(app: &FastTask, visible: &[usize]) -> Vec<Task> {
    let tm = &app.task_manager;
    if tm.selected_tasks.is_empty() {
        tm.current
            .and_then(|i| visible.get(i).copied())
            .and_then(|ri| tm.tasks.get(ri))
            .cloned()
            .into_iter()
            .collect()
    } else {
        visible
            .iter()
            .filter_map(|&ri| tm.tasks.get(ri))
            .filter(|t| tm.selected_tasks.contains(&t.id))
            .cloned()
            .collect()
    }
}

fn set_status_msg(app: &mut FastTask, msg: String) {
    app.app_state.status_msg = Some((msg, std::time::Instant::now()));
}

/// `y`: copy the Tab-selection (or the current task) into the clipboard.
fn yank(app: &mut FastTask, visible: &[usize]) {
    let tasks = action_targets(app, visible);
    let msg = match tasks.as_slice() {
        [] => return,
        [one] => format!("Yanked \"{}\"", one.title),
        many => format!("Yanked {} tasks", many.len()),
    };
    app.task_manager.clipboard = tasks;
    app.task_manager.selected_tasks.clear();
    set_status_msg(app, msg);
}

/// `p` / `Shift+P`: paste every clipboard task below / above the cursor, in order.
fn paste(app: &mut FastTask, visible: &[usize], above: bool) {
    if app.task_manager.clipboard.is_empty() {
        set_status_msg(app, "Nothing yanked — press y on a task first".into());
        return;
    }
    let tasks = &app.task_manager.tasks;
    let real_idx = app
        .task_manager
        .current
        .and_then(|i| visible.get(i).copied())
        .filter(|&ri| ri < tasks.len());
    let orders = paste_orders(tasks, real_idx, above, app.task_manager.clipboard.len());
    for (source, order) in app.task_manager.clipboard.clone().into_iter().zip(orders) {
        task_submit_paste(
            app.backend_manager.backend.clone(),
            source,
            app.project_manager.current().get_id(),
            order,
            app.backend_manager.tx.clone(),
        );
    }
}

/// `n` evenly spaced order values placed below / above `tasks[idx]`. Falls back to
/// appending at the end when there's no cursor or no room between neighbours.
pub(crate) fn paste_orders(tasks: &[Task], idx: Option<usize>, above: bool, n: usize) -> Vec<u64> {
    let append = || {
        let base = tasks.iter().map(|t| t.order).max().unwrap_or(0);
        (1..=n as u64).map(|k| base + k * ORDER_GAP).collect()
    };
    let Some(idx) = idx else {
        return append();
    };
    let here = tasks[idx].order;
    let (lo, hi) = if above {
        let prev = idx.checked_sub(1).map(|p| tasks[p].order).unwrap_or(0);
        (prev, here)
    } else {
        let next = tasks
            .get(idx + 1)
            .map(|t| t.order)
            .unwrap_or(here + ORDER_GAP * (n as u64 + 1));
        (here, next)
    };
    let step = hi.saturating_sub(lo) / (n as u64 + 1);
    if step == 0 {
        return append();
    }
    (1..=n as u64).map(|k| lo + k * step).collect()
}

/// `+` / `-`: raise / lower priority on the Tab-selection (or the current task).
fn shift_priority(app: &mut FastTask, visible: &[usize], up: bool) {
    let targets = action_targets(app, visible);
    let mut changed = Vec::new();
    for mut task in targets {
        let next = match (&task.priority, up) {
            (Priority::Low, true) => Priority::Normal,
            (Priority::Normal, true) => Priority::Urgent,
            (Priority::Urgent, false) => Priority::Normal,
            (Priority::Normal, false) => Priority::Low,
            _ => continue, // already at the top / bottom
        };
        task.priority = next;
        changed.push(task);
    }
    let msg = match changed.as_slice() {
        [] => return,
        [one] => format!("Priority: {}", one.priority),
        many => format!(
            "Priority {} on {} tasks",
            if up { "raised" } else { "lowered" },
            many.len()
        ),
    };
    // Optimistic local update so the list reflects it before the refresh lands.
    for t in &changed {
        if let Some(local) = app.task_manager.tasks.iter_mut().find(|x| x.id == t.id) {
            local.priority = t.priority.clone();
        }
    }
    let backend = app.backend_manager.backend.clone();
    let tx = app.backend_manager.tx.clone();
    // Write only the priority onto a fresh read, so a stale list copy can't
    // revert other fields saved since the list was loaded.
    let updates: Vec<(ObjectId, Priority)> =
        changed.into_iter().map(|t| (t.id, t.priority)).collect();
    crate::ui::bg::spawn(move || {
        for (id, priority) in updates {
            match backend.modify_task(id, &mut |t| t.priority = priority.clone()) {
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
    set_status_msg(app, msg);
}

fn normal_mode_keybinds(ui: &egui::Ui, app: &mut FastTask, visible: &[usize], vis_len: usize) {
    ui.input(|i| {
        let shift = i.modifiers.shift;

        // Esc peels back one layer per press: Tab-selection, then the confirmed
        // filter, then — with nothing left to clear — back to the Projects pane.
        if i.key_pressed(Key::Escape) {
            if !app.task_manager.selected_tasks.is_empty() {
                app.task_manager.selected_tasks.clear();
            } else if !app.task_manager.filter_query.is_empty() {
                app.task_manager.filter_query.clear();
            } else {
                app.app_state.window_state = WindowState::Projects;
            }
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

        // y = yank the Tab-selection (or the current task); p / Shift+P = paste below / above
        if i.key_pressed(Key::Y) {
            yank(app, visible);
        }
        if i.key_pressed(Key::P) {
            paste(app, visible, shift);
        }

        // Enter = open the current task in the Info pane (read-only view)
        if i.key_pressed(Key::Enter) && app.task_manager.current.is_some() && vis_len > 0 {
            app.app_state.window_state = WindowState::Info;
        }

        // + / - = raise / lower priority of the Tab-selection (or the current task)
        if i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals) {
            shift_priority(app, visible, true);
        }
        if i.key_pressed(Key::Minus) {
            shift_priority(app, visible, false);
        }

        // [ / ] = previous / next project without leaving the Tasks pane
        let proj_count = app.project_manager.projects.len();
        let proj_cur = app.project_manager.current_project;
        if i.key_pressed(Key::OpenBracket) && proj_cur > 0 {
            app.select_project(proj_cur - 1);
            app.task_manager.current = Some(0);
        }
        if i.key_pressed(Key::CloseBracket) && proj_cur + 1 < proj_count {
            app.select_project(proj_cur + 1);
            app.task_manager.current = Some(0);
        }

        move_cursor(i, app, vis_len, true);

        let current_task = app
            .task_manager
            .current
            .and_then(|i| visible.get(i).copied())
            .and_then(|ri| app.task_manager.tasks.get(ri))
            .cloned();
        if let Some(current_task) = current_task {
            // d = mark complete, Shift+D = hard delete
            // If there's a Tab-selection set, operate on all; otherwise on current task.
            // Ctrl+D is half-page down, not complete.
            if i.key_pressed(Key::D) && !i.modifiers.ctrl {
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

        move_cursor(i, app, vis_len, false);

        // y / p / Shift+P work in Visual mode too
        if i.key_pressed(Key::Y) {
            yank(app, visible);
        }
        if i.key_pressed(Key::P) {
            paste(app, visible, shift);
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
            if i.key_pressed(Key::D) && !i.modifiers.ctrl {
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
        // Only `order` changes; written onto a fresh read of each task.
        let updates = [
            (app.task_manager.tasks[a].id, b_o),
            (app.task_manager.tasks[b].id, a_o),
        ];
        app.task_manager.tasks.swap(a, b);
        let tx = app.backend_manager.tx.clone();
        crate::ui::bg::spawn(move || {
            for (id, order) in updates {
                match backend.modify_task(id, &mut |t| t.order = order) {
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

/// An action a mouse user triggered from a task row's hover icons (or the
/// clickable status glyph). Dispatched by `task_state` through the same
/// `task_submit_*` / mode paths the keyboard uses, giving mouse parity.
#[derive(Clone, Copy)]
pub(crate) enum RowAction {
    Edit(ObjectId),
    Complete(ObjectId),
    Delete(ObjectId),
    Status(ObjectId),
}

/// A small frameless icon button sized to sit inside a 26px task row.
fn row_icon_button(ui: &mut egui::Ui, glyph: &str, color: egui::Color32) -> egui::Response {
    ui.add(egui::Button::new(egui::RichText::new(glyph).color(color).size(14.0)).frame(false))
}

fn task_table(
    ui: &mut egui::Ui,
    tasks: &[&Task],
    selected: &mut Option<usize>,
    tab_selected: &std::collections::HashSet<ObjectId>,
) -> Option<RowAction> {
    use crate::ui::theme::colors;

    if tasks.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("No tasks here.\no / O  — new task below / above")
                    .color(colors::OVERLAY0)
                    .size(11.0),
            );
        });
        return None;
    }

    // Set by a row's hover-icon / status click; read once after the table draws.
    // `Cell` so the nested `body`/`rows` closures can capture it by shared ref
    // alongside the mutable `selected` cursor.
    let action: std::cell::Cell<Option<RowAction>> = std::cell::Cell::new(None);

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
                    // Scope every auto-generated widget id in this cell by row so
                    // the interactive status glyph / hover buttons don't clash
                    // across rows (egui_extras cells don't row-scope ids for you).
                    ui.push_id(row_index, |ui| {
                        let rect = ui.max_rect();

                        let response = ui.interact(rect, ui.id().with(row_index), Sense::click());
                        // Layer-independent hover test: stays true while the pointer is
                        // over the action buttons drawn on top, so they don't flicker.
                        let row_hovered = ui.rect_contains_pointer(rect);
                        // True once any hover icon / status glyph consumes this frame's
                        // click, so the row-select at the end doesn't also fire.
                        let mut consumed = false;

                        if is_cursor {
                            ui.painter().rect_filled(rect, 3.0, colors::BLUE);
                        } else if is_tab_sel {
                            ui.painter().rect_filled(rect, 3.0, colors::TEAL_DIM);
                        } else if row_hovered {
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
                                crate::database::models::TaskStatus::OnHold => {
                                    icons::STATUS_ON_HOLD
                                }
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
                            crate::database::models::TaskStatus::Completed => {
                                "Completed (s to change)"
                            }
                        };
                        let priority_tip = match task.priority {
                            Priority::Urgent => "Urgent priority",
                            Priority::Normal => "",
                            Priority::Low => "Low priority",
                        };
                        let render_row = |ui: &mut egui::Ui, consumed: &mut bool| {
                            // Status glyph is clickable → open the status picker for
                            // this task (mouse parity with the `s` keybind).
                            let status_resp = ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new(status_sym.to_string())
                                            .color(status_color),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text(status_tip);
                            if status_resp.clicked() {
                                action.set(Some(RowAction::Status(task.id)));
                                *consumed = true;
                            }
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!("  {}", task.title))
                                        .color(text_color),
                                )
                                .truncate(),
                            );
                            if !priority_hint.is_empty() {
                                ui.label(egui::RichText::new(priority_hint).color(priority_color))
                                    .on_hover_text(priority_tip);
                            }
                        };

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(6.0);
                            // On hover, the right edge shows the action icons (edit /
                            // complete / delete) in place of the due date; otherwise
                            // the due date. Same rect either way, so no layout jump
                            // fights the buttons for the pointer.
                            if row_hovered {
                                // On highlighted rows (BLUE / TEAL_DIM fill) the semantic
                                // icon colors would wash out — notably a BLUE edit icon on
                                // the BLUE cursor row — so fall back to MANTLE like the
                                // rest of the row's content does.
                                let (c_delete, c_complete, c_edit) = if is_highlighted {
                                    (colors::MANTLE, colors::MANTLE, colors::MANTLE)
                                } else {
                                    (colors::RED, colors::GREEN, colors::BLUE)
                                };
                                // right_to_left: first added sits rightmost.
                                if row_icon_button(ui, icons::DELETE, c_delete)
                                    .on_hover_text("Delete (Shift+D)")
                                    .clicked()
                                {
                                    action.set(Some(RowAction::Delete(task.id)));
                                    consumed = true;
                                }
                                if row_icon_button(ui, icons::STATUS_COMPLETED, c_complete)
                                    .on_hover_text("Complete (d)")
                                    .clicked()
                                {
                                    action.set(Some(RowAction::Complete(task.id)));
                                    consumed = true;
                                }
                                if row_icon_button(ui, icons::MODE_INSERT, c_edit)
                                    .on_hover_text("Edit (e)")
                                    .clicked()
                                {
                                    action.set(Some(RowAction::Edit(task.id)));
                                    consumed = true;
                                }
                            } else if let Some(ref due) = task.due {
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
                            }
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    render_row(ui, &mut consumed);
                                },
                            );
                        });

                        // Row click selects the cursor — but not when an icon / status
                        // glyph already consumed the click on this row.
                        if response.clicked() && !consumed {
                            *selected = Some(row_index);
                        }
                    });
                });
            });
        });

    action.into_inner()
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

    // --- id-clash ("red box") detection for the task table ---

    /// Recursively collect the text of any galley egui painted, so we can spot
    /// the id-clash debug overlay ("🔥 … use of widget ID …").
    fn collect_text(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
        match shape {
            egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::epaint::Shape::Vec(v) => {
                for s in v {
                    collect_text(s, out);
                }
            }
            _ => {}
        }
    }

    /// Render the real `task_table` headlessly and return any id-clash overlay
    /// strings egui painted. `hover` optionally places the pointer to exercise
    /// the on-hover action-icon path.
    fn clash_texts(hover: Option<egui::Pos2>) -> Vec<String> {
        let ctx = egui::Context::default();
        let tasks: Vec<Task> = (0..4)
            .map(|i| Task {
                title: format!("task {i}"),
                order: (i as u64 + 1) * 1000,
                status: match i {
                    0 => crate::database::models::TaskStatus::NotStarted,
                    1 => crate::database::models::TaskStatus::InProgress,
                    2 => crate::database::models::TaskStatus::OnHold,
                    _ => crate::database::models::TaskStatus::Completed,
                },
                priority: match i {
                    0 => Priority::Urgent,
                    3 => Priority::Low,
                    _ => Priority::Normal,
                },
                ..Default::default()
            })
            .collect();
        let refs: Vec<&Task> = tasks.iter().collect();
        let empty = std::collections::HashSet::new();

        // Run several frames; layout/ids stabilize after the first, and the
        // clash check compares within a single frame's used-id set.
        let mut out = Vec::new();
        for _ in 0..4 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            if let Some(p) = hover {
                input.events.push(egui::Event::PointerMoved(p));
            }
            let mut selected = Some(0usize);
            let full = ctx.run_ui(input, |ui| {
                let _ = task_table(ui, &refs, &mut selected, &empty);
            });
            out.clear();
            for cs in &full.shapes {
                collect_text(&cs.shape, &mut out);
            }
        }
        out.into_iter()
            .filter(|s| s.contains("widget ID") || s.contains("use of"))
            .collect()
    }

    #[test]
    fn task_table_has_no_id_clash_static() {
        let clashes = clash_texts(None);
        assert!(
            clashes.is_empty(),
            "id clashes on static render: {clashes:?}"
        );
    }

    #[test]
    fn task_table_has_no_id_clash_on_hover() {
        // Pointer over the first row (row height 26, table near top of panel).
        let clashes = clash_texts(Some(egui::pos2(200.0, 20.0)));
        assert!(clashes.is_empty(), "id clashes on hover: {clashes:?}");
    }

    /// Build a headless `FastTask` backed by a temp DB and pre-seeded with 4
    /// tasks. Skips `FastTask::default` and `ProjectManager::default` because both
    /// force-open the *real* database (and panic if it's locked by a running
    /// instance). Returns the app plus the TempDir guard (keep it alive).
    fn build_test_app(show_detail_pane: bool) -> (crate::ui::app::FastTask, tempfile::TempDir) {
        use crate::database::database::Db;
        use crate::ui::app::{AppState, AppType, BackendManager, FastTask, Mode, WindowState};
        use crate::ui::projects::{ProjectManager, ProjectWriter, assemble_project_list};

        let dir = tempfile::TempDir::new().unwrap();
        let db = Db::open_path(dir.path().join("t.db")).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();

        let project_manager = ProjectManager {
            projects: assemble_project_list(Vec::new()),
            current_project: 0,
            hovered_project: 0,
            writer: ProjectWriter::default(),
        };

        let tasks: Vec<Task> = (0..4)
            .map(|i| Task {
                title: format!("task {i}"),
                details: "some details".to_string(),
                order: (i as u64 + 1) * 1000,
                due: Some(polodb_core::bson::DateTime::now()),
                ..Default::default()
            })
            .collect();

        let mut app = FastTask {
            project_manager,
            task_manager: Default::default(),
            backend_manager: BackendManager {
                tx,
                rx,
                backend: std::sync::Arc::new(db),
            },
            app_state: AppState {
                mode: Mode::Normal,
                window_state: WindowState::Tasks,
                pending_delete: None,
                show_detail_pane,
                show_help: false,
                always_on_top: false,
                init: false,
                status_msg: None,
            },
            app_type: AppType::Native,
            local_share: false,
            err_ui: Default::default(),
            known_tags: Vec::new(),
            annotations: Vec::new(),
            annotation_task_id: None,
            annotation_buf: String::new(),
            annotation_cursor: None,
            tag_ui: Default::default(),
        };
        app.task_manager.tasks = tasks;
        app.task_manager.visible_cache = vec![0, 1, 2, 3];
        app.task_manager.current = Some(0);
        (app, dir)
    }

    /// Collect id-clash overlay strings from a headless render closure.
    fn clashes_from(
        mut render: impl FnMut(&mut egui::Ui),
        hover: Option<egui::Pos2>,
    ) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        for _ in 0..4 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            };
            if let Some(p) = hover {
                input.events.push(egui::Event::PointerMoved(p));
            }
            let full = ctx.run_ui(input, |ui| render(ui));
            out.clear();
            for cs in &full.shapes {
                collect_text(&cs.shape, &mut out);
            }
        }
        out.into_iter()
            .filter(|s| s.contains("widget ID") || s.contains("use of"))
            .collect()
    }

    /// Render the real full app frame (top panel + status bar + task view) via
    /// `App::ui`, and return any id-clash overlay strings.
    fn full_frame_clash_texts(show_detail_pane: bool, hover: Option<egui::Pos2>) -> Vec<String> {
        use eframe::App;
        let (mut app, _dir) = build_test_app(show_detail_pane);
        let mut frame = eframe::Frame::_new_kittest();
        clashes_from(move |ui| app.ui(ui, &mut frame), hover)
    }

    /// Render the real `task_state` (task list + bottom detail pane + mode
    /// dispatch) headlessly and return any id-clash overlay strings. This is the
    /// full "task rows" surface the user reported red boxes on. Uses a temp DB so
    /// it never touches the real one (skips `FastTask::default`, which opens it).
    fn task_state_clash_texts(show_detail_pane: bool, hover: Option<egui::Pos2>) -> Vec<String> {
        let (mut app, _dir) = build_test_app(show_detail_pane);
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        for _ in 0..4 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            };
            if let Some(p) = hover {
                input.events.push(egui::Event::PointerMoved(p));
            }
            let full = ctx.run_ui(input, |ui| {
                let _ = task_state(ui, &mut app);
            });
            out.clear();
            for cs in &full.shapes {
                collect_text(&cs.shape, &mut out);
            }
        }
        out.into_iter()
            .filter(|s| s.contains("widget ID") || s.contains("use of"))
            .collect()
    }

    #[test]
    fn full_frame_no_clash_detail_on() {
        // Detail pane on (Shift+K state) + hovering a task row: the exact
        // conditions reported as producing red boxes.
        let clashes = full_frame_clash_texts(true, Some(egui::pos2(200.0, 120.0)));
        assert!(
            clashes.is_empty(),
            "full frame clashes (detail on): {clashes:?}"
        );
    }

    #[test]
    fn full_frame_no_clash_detail_off() {
        let clashes = full_frame_clash_texts(false, Some(egui::pos2(200.0, 120.0)));
        assert!(
            clashes.is_empty(),
            "full frame clashes (detail off): {clashes:?}"
        );
    }

    #[test]
    fn task_state_no_clash_with_detail_pane() {
        let clashes = task_state_clash_texts(true, Some(egui::pos2(200.0, 40.0)));
        assert!(
            clashes.is_empty(),
            "task_state clashes (detail on): {clashes:?}"
        );
    }

    #[test]
    fn task_state_no_clash_without_detail_pane() {
        let clashes = task_state_clash_texts(false, Some(egui::pos2(200.0, 40.0)));
        assert!(
            clashes.is_empty(),
            "task_state clashes (detail off): {clashes:?}"
        );
    }

    /// Two `task_card`s in one frame (e.g. detail pane + Info pane) — their
    /// fixed `Grid`/`ScrollArea` ids would clash if not scoped per call site.
    #[test]
    fn two_task_cards_do_not_clash() {
        let ctx = egui::Context::default();
        let task = Task {
            title: "card".to_string(),
            details: "some details".to_string(),
            ..Default::default()
        };
        let mut out = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 600.0),
                )),
                ..Default::default()
            };
            let full = ctx.run_ui(input, |ui| {
                egui::Panel::top("a").show_inside(ui, |ui| task_card(ui, &task));
                egui::CentralPanel::default().show_inside(ui, |ui| task_card(ui, &task));
            });
            out.clear();
            for cs in &full.shapes {
                collect_text(&cs.shape, &mut out);
            }
        }
        let clashes: Vec<_> = out
            .into_iter()
            .filter(|s| s.contains("widget ID") || s.contains("use of"))
            .collect();
        assert!(clashes.is_empty(), "two task_cards clash: {clashes:?}");
    }

    /// Sanity check that the clash detector actually detects clashes: two grids
    /// with the same id in the *same* ui must trip it. If this ever passes, the
    /// other "no clash" tests are false negatives and can't be trusted.
    #[test]
    fn detector_catches_a_known_clash() {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            let full = ctx.run_ui(input, |ui| {
                egui::Grid::new("dup").show(ui, |ui| {
                    ui.label("a");
                    ui.end_row();
                });
                ui.add_space(40.0);
                egui::Grid::new("dup").show(ui, |ui| {
                    ui.label("b");
                    ui.end_row();
                });
            });
            out.clear();
            for cs in &full.shapes {
                collect_text(&cs.shape, &mut out);
            }
        }
        let clashes: Vec<_> = out
            .into_iter()
            .filter(|s| s.contains("widget ID") || s.contains("use of"))
            .collect();
        assert!(
            !clashes.is_empty(),
            "detector FAILED to catch a deliberate clash — other tests are unreliable"
        );
    }

    // --- pane navigation / Esc chain (full App::ui frame, real key events) ---

    /// Run one full app frame with `key` pressed (plus `modifiers`).
    fn press(app: &mut crate::ui::app::FastTask, key: egui::Key, modifiers: egui::Modifiers) {
        use eframe::App;
        let ctx = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(400.0, 300.0),
            )),
            modifiers,
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
    }

    fn window(app: &crate::ui::app::FastTask) -> &'static str {
        use crate::ui::app::WindowState;
        match app.app_state.window_state {
            WindowState::Projects => "projects",
            WindowState::Tasks => "tasks",
            WindowState::Info => "info",
        }
    }

    #[test]
    fn esc_from_projects_lands_on_tasks_without_bouncing() {
        let (mut app, _dir) = build_test_app(false);
        app.app_state.window_state = crate::ui::app::WindowState::Projects;
        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert_eq!(window(&app), "tasks");
    }

    #[test]
    fn esc_in_tasks_peels_selection_then_filter_then_goes_to_projects() {
        let (mut app, _dir) = build_test_app(false);
        let id = app.task_manager.tasks[0].id;
        app.task_manager.selected_tasks.insert(id);
        app.task_manager.filter_query = "task".into();

        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.task_manager.selected_tasks.is_empty());
        assert_eq!(app.task_manager.filter_query, "task");
        assert_eq!(window(&app), "tasks");

        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.task_manager.filter_query.is_empty());
        assert_eq!(window(&app), "tasks");

        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert_eq!(window(&app), "projects");
    }

    #[test]
    fn esc_leaving_visual_mode_stays_in_tasks() {
        let (mut app, _dir) = build_test_app(false);
        app.app_state.mode = crate::ui::app::Mode::Visual;
        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(matches!(app.app_state.mode, crate::ui::app::Mode::Normal));
        assert_eq!(window(&app), "tasks");
    }

    #[test]
    fn shift_h_l_step_through_panes_without_wrapping() {
        let (mut app, _dir) = build_test_app(false);
        let (h, l, shift) = (egui::Key::H, egui::Key::L, egui::Modifiers::SHIFT);

        press(&mut app, l, shift);
        assert_eq!(window(&app), "info");
        press(&mut app, l, shift);
        assert_eq!(window(&app), "info", "no wrap past Info");

        press(&mut app, h, shift);
        assert_eq!(window(&app), "tasks");
        press(&mut app, h, shift);
        assert_eq!(window(&app), "projects");
        press(&mut app, h, shift);
        assert_eq!(window(&app), "projects", "no wrap past Projects");
    }

    #[test]
    fn p_with_empty_clipboard_stays_in_tasks() {
        let (mut app, _dir) = build_test_app(false);
        press(&mut app, egui::Key::P, egui::Modifiers::NONE);
        assert_eq!(window(&app), "tasks");
        assert!(
            app.app_state.status_msg.is_some(),
            "shows a 'nothing yanked' hint"
        );
    }

    // --- paste ordering ---

    #[test]
    fn paste_orders_below_spreads_between_neighbours() {
        let tasks = vec![task_with_order(1000), task_with_order(4000)];
        assert_eq!(paste_orders(&tasks, Some(0), false, 2), vec![2000, 3000]);
    }

    #[test]
    fn paste_orders_above_first_task_uses_zero_as_floor() {
        let tasks = vec![task_with_order(3000)];
        assert_eq!(paste_orders(&tasks, Some(0), true, 2), vec![1000, 2000]);
    }

    #[test]
    fn paste_orders_below_last_task_leaves_room() {
        let tasks = vec![task_with_order(1000)];
        let o = paste_orders(&tasks, Some(0), false, 3);
        assert!(o.windows(2).all(|w| w[0] < w[1]) && o[0] > 1000, "{o:?}");
    }

    #[test]
    fn paste_orders_no_room_appends_at_end() {
        let tasks = vec![
            task_with_order(1000),
            task_with_order(1001),
            task_with_order(5000),
        ];
        assert_eq!(
            paste_orders(&tasks, Some(0), false, 2),
            vec![5000 + ORDER_GAP, 5000 + 2 * ORDER_GAP]
        );
    }

    #[test]
    fn paste_orders_without_cursor_appends() {
        let tasks = vec![task_with_order(1000)];
        assert_eq!(paste_orders(&tasks, None, false, 1), vec![1000 + ORDER_GAP]);
    }

    // --- new Normal-mode keys (full App::ui frame) ---

    #[test]
    fn gg_and_shift_g_jump_to_top_and_bottom() {
        let (mut app, _dir) = build_test_app(false);
        press(&mut app, egui::Key::G, egui::Modifiers::SHIFT);
        assert_eq!(app.task_manager.current, Some(3));
        press(&mut app, egui::Key::G, egui::Modifiers::NONE);
        assert_eq!(
            app.task_manager.current,
            Some(3),
            "single g does nothing yet"
        );
        press(&mut app, egui::Key::G, egui::Modifiers::NONE);
        assert_eq!(app.task_manager.current, Some(0));
    }

    #[test]
    fn g_then_other_key_cancels_gg() {
        let (mut app, _dir) = build_test_app(false);
        app.task_manager.current = Some(2);
        press(&mut app, egui::Key::G, egui::Modifiers::NONE);
        press(&mut app, egui::Key::K, egui::Modifiers::NONE);
        press(&mut app, egui::Key::G, egui::Modifiers::NONE);
        assert_eq!(
            app.task_manager.current,
            Some(1),
            "k moved; gg was cancelled"
        );
    }

    #[test]
    fn ctrl_d_pages_down_without_completing() {
        let (mut app, _dir) = build_test_app(false);
        press(&mut app, egui::Key::D, egui::Modifiers::CTRL);
        assert_eq!(app.task_manager.current, Some(3), "clamped to last row");
        assert!(
            app.task_manager
                .tasks
                .iter()
                .all(|t| t.status != TaskStatus::Completed),
            "Ctrl+D must not mark anything complete"
        );
        press(&mut app, egui::Key::U, egui::Modifiers::CTRL);
        assert_eq!(app.task_manager.current, Some(0));
    }

    #[test]
    fn arrow_keys_move_cursor() {
        let (mut app, _dir) = build_test_app(false);
        press(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        press(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(app.task_manager.current, Some(2));
        press(&mut app, egui::Key::ArrowUp, egui::Modifiers::NONE);
        assert_eq!(app.task_manager.current, Some(1));
    }

    #[test]
    fn enter_opens_info_pane() {
        let (mut app, _dir) = build_test_app(false);
        press(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(window(&app), "info");
    }

    #[test]
    fn plus_minus_shift_priority_and_clamp() {
        // One press per fresh app: a priority change triggers a background refresh
        // from the (empty) temp DB, which would wipe the in-memory list mid-test.
        let after = |start: Priority, key: egui::Key| {
            let (mut app, _dir) = build_test_app(false);
            app.task_manager.tasks[0].priority = start;
            press(&mut app, key, egui::Modifiers::NONE);
            app.task_manager.tasks[0].priority.clone()
        };
        assert_eq!(after(Priority::Low, egui::Key::Plus), Priority::Normal);
        assert_eq!(after(Priority::Normal, egui::Key::Equals), Priority::Urgent);
        assert_eq!(after(Priority::Urgent, egui::Key::Plus), Priority::Urgent);
        assert_eq!(after(Priority::Urgent, egui::Key::Minus), Priority::Normal);
        assert_eq!(after(Priority::Low, egui::Key::Minus), Priority::Low);
    }

    #[test]
    fn brackets_switch_project() {
        let (mut app, _dir) = build_test_app(false);
        assert_eq!(app.project_manager.current_project, 0);
        press(&mut app, egui::Key::CloseBracket, egui::Modifiers::NONE);
        assert_eq!(app.project_manager.current_project, 1);
        press(&mut app, egui::Key::CloseBracket, egui::Modifiers::NONE);
        assert_eq!(app.project_manager.current_project, 1, "no wrap past last");
        press(&mut app, egui::Key::OpenBracket, egui::Modifiers::NONE);
        assert_eq!(app.project_manager.current_project, 0);
        assert_eq!(window(&app), "tasks");
    }

    #[test]
    fn yank_selection_copies_in_list_order_and_clears_selection() {
        let (mut app, _dir) = build_test_app(false);
        let ids: Vec<_> = app.task_manager.tasks.iter().map(|t| t.id).collect();
        app.task_manager.selected_tasks.insert(ids[2]);
        app.task_manager.selected_tasks.insert(ids[0]);
        press(&mut app, egui::Key::Y, egui::Modifiers::NONE);
        let got: Vec<_> = app.task_manager.clipboard.iter().map(|t| t.id).collect();
        assert_eq!(got, vec![ids[0], ids[2]]);
        assert!(app.task_manager.selected_tasks.is_empty());
    }

    #[test]
    fn e_in_info_pane_enters_edit_mode() {
        let (mut app, _dir) = build_test_app(false);
        app.app_state.window_state = crate::ui::app::WindowState::Info;
        press(&mut app, egui::Key::E, egui::Modifiers::NONE);
        assert!(matches!(
            app.app_state.mode,
            crate::ui::app::Mode::Insert(Some(_))
        ));
    }

    #[test]
    fn esc_dismisses_error_banner_before_changing_pane() {
        let (mut app, _dir) = build_test_app(false);
        app.err_ui.push(
            anyhow::anyhow!("boom"),
            crate::ui::widgets::errors::ErrorSeverity::NonFatal,
        );
        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.err_ui.has_non_fatal());
        assert_eq!(window(&app), "tasks", "Esc was spent on the banner");
    }

    // --- tag completion (persistent Context so focus carries across frames) ---

    struct Harness {
        ctx: egui::Context,
        app: crate::ui::app::FastTask,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn editor(known: &[&str]) -> Self {
            let (mut app, dir) = build_test_app(false);
            app.app_state.mode = crate::ui::app::Mode::Insert(None);
            app.app_state.window_state = crate::ui::app::WindowState::Info;
            app.known_tags = known.iter().map(|t| t.to_string()).collect();
            let mut h = Self {
                ctx: egui::Context::default(),
                app,
                _dir: dir,
            };
            h.frame(vec![]);
            h.ctx
                .memory_mut(|m| m.request_focus(egui::Id::new("tag_input")));
            h.frame(vec![]);
            h
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            use eframe::App;
            let mut frame = eframe::Frame::_new_kittest();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(500.0, 900.0),
                )),
                events,
                ..Default::default()
            };
            let app = &mut self.app;
            let _ = self.ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        }

        fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
            self.frame(vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]);
            self.frame(vec![]);
        }

        fn type_text(&mut self, text: &str) {
            self.frame(vec![egui::Event::Text(text.to_string())]);
            // One idle frame so the menu is open and the focus lock is in place.
            self.frame(vec![]);
        }

        fn buf(&self) -> &str {
            &self.app.task_manager.writer.tags_buffer
        }

        fn tag_focused(&self) -> bool {
            self.ctx.memory(|m| m.has_focus(egui::Id::new("tag_input")))
        }
    }

    #[test]
    fn tab_accepts_fuzzy_match_and_keeps_focus() {
        let mut h = Harness::editor(&["work", "home", "homework"]);
        assert!(h.tag_focused(), "precondition: tag field focused");
        h.type_text("wk");
        h.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(h.buf(), "work, ");
        assert!(h.tag_focused(), "Tab must not move focus into the menu");
    }

    #[test]
    fn arrow_then_enter_accepts_second_item() {
        let mut h = Harness::editor(&["home", "homework"]);
        h.type_text("ho");
        h.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(h.buf(), "homework, ");
        assert!(h.tag_focused());
        assert!(
            matches!(h.app.app_state.mode, crate::ui::app::Mode::Insert(_)),
            "Enter on a highlighted item must not submit the form"
        );
    }

    #[test]
    fn esc_closes_menu_without_leaving_field_or_discarding() {
        let mut h = Harness::editor(&["work"]);
        h.type_text("wo");
        h.key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(h.app.task_manager.tag_menu_dismissed);
        assert!(h.tag_focused());
        assert!(matches!(
            h.app.app_state.mode,
            crate::ui::app::Mode::Insert(_)
        ));
        // With the menu closed, Tab is plain focus traversal again.
        h.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(h.buf(), "wo");
        assert!(!h.tag_focused());
    }

    #[test]
    fn already_typed_tags_are_not_suggested_again() {
        let mut h = Harness::editor(&["work", "workshop"]);
        h.type_text("work, wor");
        h.key(egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(h.buf(), "work, workshop, ");
    }

    // --- tag manager popup ---

    #[test]
    fn shift_t_opens_tag_manager_and_it_swallows_keys() {
        let (mut app, _dir) = build_test_app(false);
        press(&mut app, egui::Key::T, egui::Modifiers::SHIFT);
        assert!(app.tag_ui.open);
        assert_eq!(
            window(&app),
            "tasks",
            "Shift+T is not the plain `t` pane jump"
        );

        app.task_manager.current = Some(0);
        press(&mut app, egui::Key::J, egui::Modifiers::NONE);
        assert_eq!(
            app.task_manager.current,
            Some(0),
            "j went to the popup, not the list"
        );
        press(&mut app, egui::Key::O, egui::Modifiers::NONE);
        assert_eq!(
            app.tag_ui.mode,
            crate::ui::tags::TagUiMode::New(String::new()),
            "o starts a new tag instead of adding a task"
        );
        assert!(!matches!(
            app.app_state.mode,
            crate::ui::app::Mode::Insert(_)
        ));
        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE); // cancel name entry
        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.tag_ui.open);
        assert_eq!(window(&app), "tasks", "Esc closed the popup only");
    }

    #[test]
    fn tag_manager_rename_and_delete_flow() {
        use crate::ui::tags::TagUiMode;
        let (mut app, _dir) = build_test_app(false);
        app.tag_ui.open = true;
        app.tag_ui.rows = vec![("home".into(), 1), ("work".into(), 2)];

        press(&mut app, egui::Key::J, egui::Modifiers::NONE);
        assert_eq!(app.tag_ui.cursor, 1);
        press(&mut app, egui::Key::E, egui::Modifiers::NONE);
        assert_eq!(
            app.tag_ui.mode,
            TagUiMode::Rename {
                from: "work".into(),
                buf: "work".into()
            }
        );
        press(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert_eq!(app.tag_ui.mode, TagUiMode::Browse);
        assert!(
            app.tag_ui.open,
            "Esc in the name field cancels, doesn't close"
        );

        press(&mut app, egui::Key::D, egui::Modifiers::NONE);
        assert_eq!(app.tag_ui.mode, TagUiMode::ConfirmDelete("work".into()));
        press(&mut app, egui::Key::N, egui::Modifiers::NONE);
        assert_eq!(app.tag_ui.mode, TagUiMode::Browse);
    }

    #[test]
    fn editing_a_task_loads_its_notes() {
        let (mut app, _dir) = build_test_app(false);
        let id = app.task_manager.tasks[2].id;
        app.app_state.mode = crate::ui::app::Mode::Insert(Some(id));
        app.app_state.window_state = crate::ui::app::WindowState::Info;
        press(&mut app, egui::Key::F1, egui::Modifiers::NONE);
        assert_eq!(
            app.annotation_task_id,
            Some(id),
            "editor fetched this task's notes"
        );
    }

    #[test]
    fn completing_recurring_task_creates_one_next_occurrence_even_when_repeated() {
        use crate::database::database::Db;
        let dir = tempfile::TempDir::new().unwrap();
        let backend: Backend = std::sync::Arc::new(Db::open_path(dir.path().join("t.db")).unwrap());
        let id = backend
            .create_task(Task {
                title: "water plants".into(),
                order: 1000,
                recurrence: Some(Recurrence::Weekly),
                ..Default::default()
            })
            .unwrap();
        // Bulk path and single path share apply_status; completing twice must
        // not spawn a second occurrence.
        apply_status(&backend, id, &TaskStatus::Completed).unwrap();
        apply_status(&backend, id, &TaskStatus::Completed).unwrap();
        let all = backend
            .get_tasks(crate::database::ProjectEntry::All)
            .unwrap();
        let open: Vec<_> = all
            .iter()
            .filter(|t| t.status != TaskStatus::Completed)
            .collect();
        assert_eq!(all.len(), 2);
        assert_eq!(open.len(), 1);
        assert!(open[0].due.is_some(), "next occurrence is scheduled");
    }

    #[test]
    fn save_and_done_on_recurring_task_schedules_next() {
        use crate::database::database::Db;
        let dir = tempfile::TempDir::new().unwrap();
        let backend: Backend = std::sync::Arc::new(Db::open_path(dir.path().join("t.db")).unwrap());
        let id = backend
            .create_task(Task {
                title: "old".into(),
                order: 1000,
                recurrence: Some(Recurrence::Daily),
                ..Default::default()
            })
            .unwrap();
        let writer = TaskWriter {
            title_buffer: "renamed".into(),
            status: TaskStatus::Completed,
            recurrence: Some(Recurrence::Daily),
            ..Default::default()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        task_submit_edit(backend.clone(), writer, id, tx);
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();

        let all = backend
            .get_tasks(crate::database::ProjectEntry::All)
            .unwrap();
        let next: Vec<_> = all
            .iter()
            .filter(|t| t.status != TaskStatus::Completed)
            .collect();
        assert_eq!(all.len(), 2);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].title, "renamed", "built from the saved values");
    }
}
