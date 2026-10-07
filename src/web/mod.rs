//! Browser client for the LAN share: a task list + details view over the
//! desktop app's `/api/v1`, drawn with the same `ui::view` renderers as the
//! desktop panes. Read-only unless the desktop turns on "allow edits"; then it
//! can add, edit, complete and delete tasks and notes. Built to wasm by
//! `scripts/build-web.sh` and served by `local_share::server`.
//!
//! Data flow mirrors the desktop's `thread_sync`: fetch callbacks and the
//! event stream push [`Msg`]s into a channel that `update` drains each frame.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use bson::oid::ObjectId;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

use crate::database::models::{
    ActivityEntry, Annotation, Priority, ProjectEntry, Task, TaskStatus,
};
use crate::local_share::api::{
    NAME_HEADER, NAME_MAX, NewNote, NewTask, ShareInfo, TOKEN_HEADER, TaskPatch, decode_name,
    encode_name,
};
use crate::ui::theme::{self, colors, icons};
use crate::ui::view::{self, RowIcon};
use crate::ui::widgets::common;

/// Refetch this often even without a push, in case the event stream is down.
const POLL_SECS: f64 = 30.0;
/// At or above this width (points) the list and details sit side by side.
const TWO_COLUMN_WIDTH: f32 = 720.0;
/// Width of the activity column when it fits beside the list and details.
const ACTIVITY_WIDTH: f32 = 300.0;
/// Wait before retrying after a failed read.
const RETRY_MS: f64 = 3000.0;
/// How long a failed edit's message stays up.
const TOAST_MS: f64 = 5000.0;
/// Where this device remembers who's editing.
const NAME_KEY: &str = "fast-task.name";

/// Mount the app on `#the_canvas_id`. Called from `main` on module load.
pub fn start() {
    // eframe's warnings and errors (e.g. WebGL trouble) go to the browser console.
    let _ = eframe::WebLogger::init(log::LevelFilter::Warn);
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .and_then(|w| w.document())
            .expect("no document");
        let canvas = document
            .get_element_by_id("the_canvas_id")
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("no #the_canvas_id canvas");
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(WebApp::new(cc)))),
            )
            .await;
        if let Some(loading) = document.get_element_by_id("loading") {
            match result {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_inner_html(&format!(
                    "<p>FastTask couldn't start in this browser.</p><pre>{e:?}</pre>"
                )),
            }
        }
    });
}

enum Msg {
    Project(u64, ProjectEntry),
    Tasks(u64, Vec<Task>),
    Share(ShareInfo),
    Activity(Vec<ActivityEntry>),
    Notes(ObjectId, Vec<Annotation>),
    /// The desktop wrote something; refetch.
    Changed,
    /// Event stream opened (`true`) or dropped and is retrying (`false`).
    Live(bool),
    /// The link no longer works (401): replaces the view — only a new link helps.
    Expired(String),
    /// A read failed (network, server error): a banner over the page while it
    /// retries; whatever was already shown stays.
    Unreachable(String),
    /// An edit went through; refetch without waiting for the event stream.
    Wrote,
    /// An edit failed: a transient banner, the view stays.
    WriteFailed(String),
}

/// An edit held back until the person says who they are.
struct PendingWrite {
    method: String,
    url: String,
    body: Option<Vec<u8>>,
}

/// The task being edited in the details pane.
struct Draft {
    id: ObjectId,
    title: String,
    details: String,
}

struct WebApp {
    token: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    project: Option<ProjectEntry>,
    /// `None` until the first fetch lands.
    tasks: Option<Vec<Task>>,
    notes: HashMap<ObjectId, Vec<Annotation>>,
    selected: Option<ObjectId>,
    show_completed: bool,
    can_edit: bool,
    /// How the desktop's edits are labelled in the activity log.
    host_name: String,
    /// Who's editing from this device (remembered in localStorage).
    name: Option<String>,
    /// "Who's editing?" prompt text; `Some` while the prompt is open.
    name_prompt: Option<String>,
    pending: Option<PendingWrite>,
    /// Asked for a name already this visit (so Cancel isn't nagged again).
    name_asked: bool,
    show_activity: bool,
    activity: Vec<ActivityEntry>,
    live: bool,
    /// The stream has opened at least once ("connecting" vs. "reconnecting").
    was_live: bool,
    error: Option<String>,
    toast: Option<(String, f64)>,
    new_task: String,
    new_note: String,
    draft: Option<Draft>,
    /// Task awaiting a second tap on Delete.
    confirm_delete: Option<ObjectId>,
    /// Bumped per refresh; responses from an older one are dropped.
    generation: u64,
    last_refresh_ms: f64,
    /// The live stream. Reopened if the browser gives up on it (it does when
    /// the server refuses it, e.g. while restarting).
    events: Option<web_sys::EventSource>,
    /// Why reads are failing, for the banner; `None` when all is well.
    unreachable: Option<String>,
    /// When to retry after a failure (ms since epoch).
    retry_at: Option<f64>,
}

impl WebApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        // Phones: egui's 13pt body text reads small at arm's length.
        if web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|w| w.as_f64())
            .is_some_and(|w| w < 600.0)
        {
            cc.egui_ctx.set_zoom_factor(1.2);
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let token = share_token();
        let events = token
            .as_deref()
            .and_then(|t| subscribe(t, tx.clone(), cc.egui_ctx.clone()));
        let mut app = Self {
            token,
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            project: None,
            tasks: None,
            notes: HashMap::new(),
            selected: None,
            show_completed: false,
            can_edit: false,
            host_name: "Host".into(),
            name: load_name(),
            name_prompt: None,
            pending: None,
            name_asked: false,
            show_activity: false,
            activity: Vec::new(),
            live: false,
            was_live: false,
            error: None,
            toast: None,
            new_task: String::new(),
            new_note: String::new(),
            draft: None,
            confirm_delete: None,
            generation: 0,
            last_refresh_ms: 0.0,
            events,
            unreachable: None,
            retry_at: None,
        };
        app.refresh();
        app
    }

    /// Refetch whether edits are allowed, the current project, then its tasks,
    /// and the open task's notes.
    fn refresh(&mut self) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.generation += 1;
        self.last_refresh_ms = js_sys::Date::now();
        let generation = self.generation;
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        get_json::<ShareInfo>("api/v1/share", &token, &tx, &ctx, {
            let tx = tx.clone();
            move |info| {
                let _ = tx.send(Msg::Share(info));
            }
        });
        get_json::<ProjectEntry>("api/v1/current-project", &token, &tx, &ctx, {
            let (tx, ctx, token) = (tx.clone(), ctx.clone(), token.clone());
            move |project| {
                let url = format!("api/v1/tasks?project_id={}", project_param(&project));
                let _ = tx.send(Msg::Project(generation, project));
                get_json::<Vec<Task>>(&url, &token, &tx, &ctx, {
                    let tx = tx.clone();
                    move |tasks| {
                        let _ = tx.send(Msg::Tasks(generation, tasks));
                    }
                });
            }
        });
        if let Some(id) = self.selected {
            self.fetch_notes(id);
        }
        if self.show_activity {
            self.fetch_activity();
        }
    }

    fn fetch_activity(&self) {
        let Some(token) = &self.token else {
            return;
        };
        let tx = self.tx.clone();
        get_json::<Vec<ActivityEntry>>("api/v1/changes", token, &self.tx, &self.ctx, move |log| {
            let _ = tx.send(Msg::Activity(log));
        });
    }

    fn fetch_notes(&self, id: ObjectId) {
        let Some(token) = &self.token else {
            return;
        };
        let tx = self.tx.clone();
        get_json::<Vec<Annotation>>(
            &format!("api/v1/task/{}/annotations", id.to_hex()),
            token,
            &self.tx,
            &self.ctx,
            move |notes| {
                let _ = tx.send(Msg::Notes(id, notes));
            },
        );
    }

    /// Send an edit. The result arrives as `Msg::Wrote` / `Msg::WriteFailed`.
    /// The first edit on a device asks who's editing, then goes through.
    fn write(&mut self, method: &str, url: String, body: Option<Vec<u8>>) {
        let Some(name) = self.name.clone() else {
            self.pending = Some(PendingWrite {
                method: method.to_string(),
                url,
                body,
            });
            self.name_prompt = Some(String::new());
            return;
        };
        let Some(token) = &self.token else {
            return;
        };
        let name = encode_name(&name);
        let request = ehttp::Request {
            method: method.to_string(),
            url,
            body: body.unwrap_or_default(),
            // Set by hand: ehttp's `json` helper is behind a feature we don't need.
            headers: ehttp::Headers::new(&[
                (TOKEN_HEADER, token.as_str()),
                (NAME_HEADER, name.as_str()),
                ("Content-Type", "application/json"),
            ]),
            ..ehttp::Request::get("")
        };
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        ehttp::fetch(request, move |result| {
            let msg = match result {
                Ok(res) if res.ok => Msg::Wrote,
                Ok(res) if res.status == 403 => {
                    Msg::WriteFailed("Editing was turned off on the desktop.".into())
                }
                Ok(res) => Msg::WriteFailed(format!(
                    "Couldn't save ({}): {}",
                    res.status,
                    error_text(&res)
                )),
                Err(e) => {
                    log::warn!("write failed: {e}");
                    Msg::WriteFailed("Can't reach the desktop app.".into())
                }
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    fn patch(&mut self, id: ObjectId, patch: TaskPatch) {
        self.write(
            "PATCH",
            format!("api/v1/task/{}", id.to_hex()),
            serde_json::to_vec(&patch).ok(),
        );
    }

    fn drain(&mut self) {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Project(generation, project) if generation == self.generation => {
                    self.project = Some(project);
                }
                Msg::Tasks(generation, mut tasks) if generation == self.generation => {
                    tasks.sort_by_key(|t| t.order);
                    if self
                        .selected
                        .is_some_and(|id| !tasks.iter().any(|t| t.id == id))
                    {
                        self.selected = None;
                    }
                    self.tasks = Some(tasks);
                    self.error = None;
                    self.unreachable = None;
                    self.retry_at = None;
                }
                Msg::Project(..) | Msg::Tasks(..) => {}
                Msg::Activity(log) => self.activity = log,
                Msg::Share(info) => {
                    self.can_edit = info.can_edit;
                    self.host_name = info.host_name;
                    // Ask who's editing as soon as editing is possible, rather
                    // than in the middle of someone's first edit.
                    if info.can_edit && self.name.is_none() && !self.name_asked {
                        self.name_asked = true;
                        self.name_prompt.get_or_insert_with(String::new);
                    }
                    if !info.can_edit {
                        self.draft = None;
                        self.confirm_delete = None;
                    }
                }
                Msg::Notes(id, notes) => {
                    self.notes.insert(id, notes);
                }
                Msg::Changed | Msg::Wrote => changed = true,
                Msg::Live(live) => {
                    // Catch up on anything missed while the stream was down.
                    changed |= live && !self.live;
                    self.live = live;
                    self.was_live |= live;
                }
                Msg::Expired(e) => self.error = Some(e),
                Msg::Unreachable(e) => {
                    self.unreachable = Some(e);
                    self.retry_at.get_or_insert(js_sys::Date::now() + RETRY_MS);
                }
                Msg::WriteFailed(e) => {
                    self.toast = Some((e, js_sys::Date::now()));
                    // The edit may have raced a change on the desktop; resync.
                    changed = true;
                }
            }
        }
        if changed {
            self.notes.clear();
            self.refresh();
        }
    }

    /// Tasks the desktop would show: not waiting, and completed ones only on request.
    fn visible(&self) -> Vec<&Task> {
        let now_ms = js_sys::Date::now() as i64;
        self.tasks
            .iter()
            .flatten()
            .filter(|t| !view::is_waiting(t, now_ms))
            .filter(|t| self.show_completed || t.status != TaskStatus::Completed)
            .collect()
    }

    fn select(&mut self, id: Option<ObjectId>) {
        if self.selected != id {
            self.draft = None;
            self.confirm_delete = None;
            self.new_note.clear();
        }
        self.selected = id;
        if let Some(id) = id
            && !self.notes.contains_key(&id)
        {
            self.fetch_notes(id);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("web_top").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                common::heading(ui, "FastTask");
                if let Some(project) = &self.project {
                    ui.label(egui::RichText::new(project.to_string()).color(colors::SUBTEXT0));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(
                            self.show_activity,
                            view::selectable_icon(icons::ACTIVITY, self.show_activity),
                        )
                        .on_hover_text("Activity: who changed what")
                        .clicked()
                    {
                        self.show_activity = !self.show_activity;
                        if self.show_activity {
                            self.fetch_activity();
                        }
                    }
                    let (dot, text, tip) = if self.live {
                        (
                            colors::GREEN,
                            "live",
                            "Updates as tasks change on the desktop",
                        )
                    } else if self.was_live {
                        (
                            colors::YELLOW,
                            "reconnecting",
                            "Lost the live connection; retrying (and refreshing every 30 s)",
                        )
                    } else {
                        (
                            colors::OVERLAY1,
                            "connecting",
                            "Connecting to the desktop app…",
                        )
                    };
                    ui.label(egui::RichText::new(text).color(dot).size(11.0))
                        .on_hover_text(tip);
                    // Painted, not "●": that glyph isn't in the bundled fonts and
                    // drew as an empty box, which read as "not connected".
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 3.5, dot);
                    let (mode, color) = if self.can_edit {
                        ("can edit", colors::PEACH)
                    } else {
                        ("read-only", colors::OVERLAY1)
                    };
                    ui.label(egui::RichText::new(mode).color(color).size(11.0));
                    if self.can_edit
                        && let Some(name) = &self.name
                        && ui
                            .add(
                                egui::Label::new(
                                    egui::RichText::new(format!("as {name}"))
                                        .color(colors::BLUE)
                                        .size(11.0),
                                )
                                .sense(egui::Sense::click()),
                            )
                            .on_hover_text("Change your name")
                            .clicked()
                    {
                        self.name_prompt = Some(name.clone());
                    }
                });
            });
            ui.add_space(2.0);
        });
    }

    /// A slim banner while reads fail; the page underneath keeps its last data.
    fn unreachable_banner(&mut self, ui: &mut egui::Ui) {
        let Some(text) = &self.unreachable else {
            return;
        };
        let text = text.clone();
        let mut retry = false;
        egui::Panel::top("web_unreachable").show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(text).color(colors::PEACH).size(12.0));
                retry = ui.small_button("Retry now").clicked();
            });
        });
        if retry {
            self.retry_at = None;
            self.reconnect_if_closed();
            self.refresh();
        }
    }

    /// Reopen the live stream if the browser has given up on it. It retries
    /// by itself after a dropped connection, but not after a refused one.
    fn reconnect_if_closed(&mut self) {
        const CLOSED: u16 = 2;
        let closed = self
            .events
            .as_ref()
            .is_none_or(|e| e.ready_state() == CLOSED);
        if let (true, Some(token)) = (closed, &self.token) {
            if let Some(old) = self.events.take() {
                old.close();
            }
            self.events = subscribe(token, self.tx.clone(), self.ctx.clone());
        }
    }

    /// "Who's editing?" — asked before this device's first edit, and when
    /// renaming. Continue sends the edit that was waiting for it.
    fn name_prompt(&mut self, ui: &mut egui::Ui) {
        let Some(buf) = &mut self.name_prompt else {
            return;
        };
        let (mut done, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("name_prompt")).show(ui.ctx(), |ui| {
            ui.set_max_width(280.0);
            common::heading(ui, "Who's editing?");
            ui.label(
                egui::RichText::new("Your name shows next to your changes in the activity log.")
                    .color(colors::SUBTEXT0)
                    .size(12.0),
            );
            ui.add_space(6.0);
            let resp = ui.add(
                egui::TextEdit::singleline(buf)
                    .hint_text("Your name")
                    .char_limit(NAME_MAX)
                    .desired_width(f32::INFINITY),
            );
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            done |= resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                done |= common::secondary_button(
                    ui,
                    egui::RichText::new("Continue").color(colors::BLUE),
                )
                .clicked();
                cancel = common::secondary_button(ui, "Cancel").clicked();
            });
        });
        cancel |= modal.should_close() && !done;
        let name = decode_name(&encode_name(buf));
        if done && let Some(name) = name {
            save_name(&name);
            self.name = Some(name);
            self.name_prompt = None;
            if let Some(PendingWrite { method, url, body }) = self.pending.take() {
                self.write(&method, url, body);
            }
        } else if cancel {
            self.name_prompt = None;
            self.pending = None;
        }
    }

    /// Who changed what, newest first, labelled from this device's view: its own
    /// edits are "You", the desktop's are the host's name.
    /// A person as this device sees them: its own edits are "You", the desktop's
    /// (no name) are the host's.
    fn author_label(&self, who: &Option<String>) -> (String, egui::Color32) {
        match who {
            // "Host (host)" if the desktop never set a name.
            None if self.host_name == "Host" => ("Host".into(), colors::LAVENDER),
            None => (format!("{} (host)", self.host_name), colors::LAVENDER),
            Some(name) if Some(name) == self.name.as_ref() => ("You".into(), colors::BLUE),
            Some(name) => (name.clone(), view::person_color(name)),
        }
    }

    fn activity_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            common::heading(ui, "Activity");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if view::small_icon_button(ui, icons::CLOSE, colors::TEXT)
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    self.show_activity = false;
                }
            });
        });
        ui.separator();
        if self.activity.is_empty() {
            hint(ui, "No changes yet.");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("web_activity")
            .auto_shrink(false)
            .show(ui, |ui| {
                view::activity_list(ui, &self.activity, |who| self.author_label(who));
            });
    }

    /// The activity log sliding over from the right, for screens too narrow to
    /// give it a column. Tap outside, ✕, or Esc closes it.
    fn activity_drawer(&mut self, ctx: &egui::Context, body: egui::Rect) {
        let width = (body.width() - 48.0).clamp(body.width().min(240.0), ACTIVITY_WIDTH + 40.0);
        let backdrop = egui::Area::new(egui::Id::new("activity_backdrop"))
            .order(egui::Order::Middle)
            .fixed_pos(body.min)
            .show(ctx, |ui| {
                let (rect, response) = ui.allocate_exact_size(body.size(), egui::Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, egui::Color32::from_black_alpha(110));
                response
            });
        if backdrop.inner.clicked() {
            self.show_activity = false;
        }
        egui::Area::new(egui::Id::new("activity_drawer"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(body.right() - width, body.top()))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors::BASE)
                    .stroke(egui::Stroke::new(1.0_f32, colors::SURFACE1))
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.set_height(body.height() - 24.0);
                        self.activity_view(ui);
                    });
            });
    }

    fn toast(&mut self, ui: &mut egui::Ui) {
        let Some((text, at)) = &self.toast else {
            return;
        };
        if js_sys::Date::now() - at > TOAST_MS {
            self.toast = None;
            return;
        }
        let text = text.clone();
        egui::Panel::bottom("web_toast").show_inside(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(text).color(colors::RED));
            });
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(TOAST_MS as u64));
    }

    fn task_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            common::field_label(ui, "Tasks");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.toggle_value(
                    &mut self.show_completed,
                    egui::RichText::new(format!("{} completed", icons::STATUS_COMPLETED))
                        .size(11.0),
                )
                .on_hover_text("Show completed tasks");
            });
        });
        ui.add_space(2.0);

        if self.can_edit {
            let add = common::add_row(ui, &mut self.new_task, "Add a task…");
            if add && !self.new_task.trim().is_empty() {
                let body = NewTask {
                    title: std::mem::take(&mut self.new_task),
                };
                self.write("POST", "api/v1/task".into(), serde_json::to_vec(&body).ok());
            }
            ui.add_space(4.0);
        }

        if self.tasks.is_none() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| ui.add(egui::Spinner::new()));
            return;
        }
        let mut clicked = None;
        let mut icon = None;
        let visible = self.visible();
        egui::ScrollArea::vertical()
            .id_salt("web_tasks")
            .auto_shrink(false)
            .show(ui, |ui| {
                if visible.is_empty() {
                    hint(ui, "No tasks here.");
                }
                for task in &visible {
                    let selected = self.selected == Some(task.id);
                    let (row, row_icon) = view::task_row(ui, task, selected, self.can_edit);
                    if let Some(row_icon) = row_icon {
                        icon = Some(((*task).clone(), row_icon));
                    } else if row.clicked() {
                        clicked = Some(task.id);
                    }
                }
            });
        if clicked.is_some() {
            self.select(clicked);
        }
        match icon {
            Some((task, RowIcon::Edit)) => {
                self.select(Some(task.id));
                self.start_edit(&task);
            }
            Some((task, RowIcon::Complete)) => self.patch(
                task.id,
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            ),
            None => {}
        }
    }

    fn details(&mut self, ui: &mut egui::Ui, back_button: bool) {
        if back_button && common::secondary_button(ui, "‹ Tasks").clicked() {
            self.select(None);
            return;
        }
        let task = self
            .selected
            .and_then(|id| self.tasks.iter().flatten().find(|t| t.id == id))
            .cloned();
        let Some(task) = task else {
            ui.centered_and_justified(|ui| hint(ui, "Select a task to see its details."));
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("web_details")
            .auto_shrink(false)
            .show(ui, |ui| {
                if self.draft.as_ref().is_some_and(|d| d.id == task.id) {
                    self.edit_form(ui, &task);
                } else {
                    view::task_card(ui, &task);
                    if self.can_edit {
                        ui.add_space(8.0);
                        self.task_actions(ui, &task);
                    }
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                self.notes_section(ui, task.id);
            });
    }

    /// Status / priority pickers and Edit / Delete, under the task card.
    fn task_actions(&mut self, ui: &mut egui::Ui, task: &Task) {
        common::field_label(ui, "Status");
        ui.horizontal_wrapped(|ui| {
            for status in [
                TaskStatus::NotStarted,
                TaskStatus::InProgress,
                TaskStatus::OnHold,
                TaskStatus::Completed,
            ] {
                let text = egui::RichText::new(format!(
                    "{} {}",
                    view::status_icon(&status),
                    view::status_name(&status)
                ))
                .color(on_selected(
                    task.status == status,
                    theme::status_color(&status),
                ));
                if ui.selectable_label(task.status == status, text).clicked()
                    && task.status != status
                {
                    self.patch(
                        task.id,
                        TaskPatch {
                            status: Some(status),
                            ..Default::default()
                        },
                    );
                }
            }
        });
        ui.add_space(4.0);
        common::field_label(ui, "Priority");
        ui.horizontal_wrapped(|ui| {
            for priority in [Priority::Urgent, Priority::Normal, Priority::Low] {
                let text = egui::RichText::new(priority.to_string()).color(on_selected(
                    task.priority == priority,
                    theme::priority_color(&priority),
                ));
                if ui
                    .selectable_label(task.priority == priority, text)
                    .clicked()
                    && task.priority != priority
                {
                    self.patch(
                        task.id,
                        TaskPatch {
                            priority: Some(priority),
                            ..Default::default()
                        },
                    );
                }
            }
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if common::secondary_button(ui, format!("{}  Edit", icons::MODE_INSERT)).clicked() {
                self.start_edit(task);
            }
            if self.confirm_delete == Some(task.id) {
                ui.label(egui::RichText::new("Delete this task?").color(colors::RED));
                if common::danger_button(ui, "Delete").clicked() {
                    self.confirm_delete = None;
                    self.write("DELETE", format!("api/v1/task/{}", task.id.to_hex()), None);
                }
                if common::secondary_button(ui, "Keep").clicked() {
                    self.confirm_delete = None;
                }
            } else if common::secondary_button(
                ui,
                egui::RichText::new(format!("{}  Delete", icons::DELETE)).color(colors::RED),
            )
            .clicked()
            {
                self.confirm_delete = Some(task.id);
            }
        });
    }

    fn start_edit(&mut self, task: &Task) {
        self.draft = Some(Draft {
            id: task.id,
            title: task.title.clone(),
            details: task.details.clone(),
        });
    }

    /// Title + details editor. Saving is a button: phones can't type Shift+Enter,
    /// so Enter in Details is a newline here.
    fn edit_form(&mut self, ui: &mut egui::Ui, task: &Task) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        common::field_label(ui, "Title");
        ui.add(egui::TextEdit::singleline(&mut draft.title).desired_width(f32::INFINITY));
        ui.add_space(4.0);
        common::field_label(ui, "Details");
        let details = egui::TextEdit::multiline(&mut draft.details)
            .desired_width(f32::INFINITY)
            .desired_rows(if task.code { 8 } else { 4 });
        ui.add(if task.code || task.language.is_some() {
            details.code_editor()
        } else {
            details.hint_text("Additional context…")
        });
        ui.add_space(6.0);
        let (mut save, mut cancel) = (false, false);
        ui.horizontal(|ui| {
            save = common::secondary_button(
                ui,
                egui::RichText::new(format!("{}  Save", icons::SAVE)).color(colors::BLUE),
            )
            .clicked();
            cancel = common::secondary_button(ui, "Cancel").clicked();
        });
        if save && !draft.title.trim().is_empty() {
            let Draft { title, details, .. } = self.draft.take().unwrap();
            let patch = TaskPatch {
                title: (title != task.title).then_some(title),
                details: (details != task.details).then_some(details),
                ..Default::default()
            };
            if patch.title.is_some() || patch.details.is_some() {
                self.patch(task.id, patch);
            }
        } else if save {
            self.toast = Some(("A task needs a title.".into(), js_sys::Date::now()));
        } else if cancel {
            self.draft = None;
        }
    }

    fn notes_section(&mut self, ui: &mut egui::Ui, task_id: ObjectId) {
        common::field_label(ui, "Notes");
        let Some(notes) = self.notes.get(&task_id) else {
            ui.add(egui::Spinner::new());
            return;
        };
        if !self.can_edit {
            view::notes_list(ui, notes, |author| Some(self.author_label(author)));
            return;
        }
        if notes.is_empty() {
            view::notes_list(ui, notes, |_| None);
        }
        let mut delete = None;
        for note in notes {
            let card =
                view::note_card(ui, note, Some(self.author_label(&note.author)), false, true);
            if card.delete {
                delete = Some(note.id);
            }
        }
        if let Some(id) = delete {
            self.write("DELETE", format!("api/v1/annotation/{}", id.to_hex()), None);
        }

        let add = common::add_row(ui, &mut self.new_note, "Add a note…");
        if add && !self.new_note.trim().is_empty() {
            let body = NewNote {
                content: std::mem::take(&mut self.new_note),
            };
            self.write(
                "POST",
                format!("api/v1/task/{}/annotations", task_id.to_hex()),
                serde_json::to_vec(&body).ok(),
            );
        }
    }
}

impl eframe::App for WebApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain();
        let now = js_sys::Date::now();
        if self.retry_at.is_some_and(|at| now >= at) {
            self.retry_at = None;
            self.reconnect_if_closed();
            self.refresh();
        } else if now - self.last_refresh_ms > POLL_SECS * 1000.0 {
            self.reconnect_if_closed();
            self.refresh();
        }
        if let Some(at) = self.retry_at {
            ui.ctx()
                .request_repaint_after(
                    std::time::Duration::from_millis((at - now).max(0.0) as u64),
                );
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(POLL_SECS));

        self.top_bar(ui);
        self.unreachable_banner(ui);
        self.toast(ui);
        self.name_prompt(ui);

        if self.token.is_none() || self.error.is_some() {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    let text = self.error.clone().unwrap_or_else(|| {
                        "This link is missing its access token.\n\
                         Open the link (or scan the QR code) from the desktop app's share popup."
                            .into()
                    });
                    ui.label(egui::RichText::new(text).color(colors::PEACH));
                });
            });
            return;
        }

        if self.show_activity
            && self.name_prompt.is_none()
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.show_activity = false;
        }
        let body = ui.available_rect_before_wrap();
        let width = body.width();
        let wide = width >= TWO_COLUMN_WIDTH;
        // Activity gets its own column when the list and details keep their
        // room; otherwise it slides over the right side as a drawer.
        let docked = self.show_activity && width >= TWO_COLUMN_WIDTH + ACTIVITY_WIDTH;
        if docked {
            egui::Panel::right("web_activity_panel")
                .resizable(true)
                .default_size(ACTIVITY_WIDTH)
                .show_inside(ui, |ui| self.activity_view(ui));
        }
        if wide {
            egui::Panel::left("web_list")
                .resizable(true)
                .default_size(360.0)
                .show_inside(ui, |ui| self.task_list(ui));
            egui::CentralPanel::default().show_inside(ui, |ui| self.details(ui, false));
        } else {
            egui::CentralPanel::default().show_inside(ui, |ui| {
                if self.selected.is_some() {
                    self.details(ui, true);
                } else {
                    self.task_list(ui);
                }
            });
        }
        if self.show_activity && !docked {
            let ctx = ui.ctx().clone();
            self.activity_drawer(&ctx, body);
        }
    }
}

/// Text color on a selectable label: the selected fill is a light blue that
/// washes out colored text, so selected labels use the dark base color.
fn on_selected(selected: bool, color: egui::Color32) -> egui::Color32 {
    if selected { colors::MANTLE } else { color }
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .color(colors::OVERLAY1)
            .size(12.0)
            .italics(),
    );
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The name this device last edited under.
fn load_name() -> Option<String> {
    decode_name(&encode_name(&storage()?.get_item(NAME_KEY).ok()??))
}

fn save_name(name: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(NAME_KEY, name);
    }
}

/// The `t` query parameter of the page URL.
fn share_token() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("t="))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// The `/tasks?project_id=` value for a project entry.
fn project_param(project: &ProjectEntry) -> String {
    match project {
        ProjectEntry::All => "All".into(),
        ProjectEntry::None => "None".into(),
        ProjectEntry::Project(p) => p.id.to_hex(),
    }
}

/// The `error` field of an API error body, else the raw text.
fn error_text(res: &ehttp::Response) -> String {
    #[derive(serde::Deserialize)]
    struct ErrorBody {
        error: String,
    }
    serde_json::from_slice::<ErrorBody>(&res.bytes)
        .map(|b| b.error)
        .unwrap_or_else(|_| res.text().unwrap_or_default().to_string())
}

/// GET `url` with the share token and decode the JSON body. Failures become
/// `Msg::Expired` / `Msg::Unreachable` with a message meant for the person
/// holding the phone.
fn get_json<T: serde::de::DeserializeOwned>(
    url: &str,
    token: &str,
    tx: &Sender<Msg>,
    ctx: &egui::Context,
    on_ok: impl FnOnce(T) + Send + 'static,
) {
    let mut request = ehttp::Request::get(url);
    request.headers.insert(TOKEN_HEADER, token);
    let (tx, ctx, url) = (tx.clone(), ctx.clone(), url.to_string());
    ehttp::fetch(request, move |result| {
        let outcome = match result {
            Ok(res) if res.status == 401 => Err(Msg::Expired(
                "This share link has expired — sharing was restarted on the desktop.\n\
                 Open the new link or scan the new QR code."
                    .to_string(),
            )),
            Ok(res) if !res.ok => Err(Msg::Unreachable(format!(
                "The desktop app returned an error ({}): {}",
                res.status,
                error_text(&res)
            ))),
            Ok(res) => serde_json::from_slice::<T>(&res.bytes).map_err(|e| {
                Msg::Unreachable(format!("Couldn't read the desktop app's response: {e}"))
            }),
            Err(e) => {
                // The details only go to the console; the banner keeps it short.
                log::warn!("GET {url} failed: {e}");
                Err(Msg::Unreachable(
                    "Can't reach the desktop app — retrying…".to_string(),
                ))
            }
        };
        match outcome {
            Ok(value) => on_ok(value),
            Err(msg) => {
                let _ = tx.send(msg);
            }
        }
        ctx.request_repaint();
    });
}

/// Open the `/events` stream. The browser reconnects on its own after a drop;
/// `Msg::Live` tracks which state it's in.
fn subscribe(token: &str, tx: Sender<Msg>, ctx: egui::Context) -> Option<web_sys::EventSource> {
    let source = web_sys::EventSource::new(&format!("api/v1/events?t={token}")).ok()?;
    let send = move |msg: Msg| {
        let _ = tx.send(msg);
        ctx.request_repaint();
    };

    let on_changed = Closure::<dyn FnMut(web_sys::MessageEvent)>::new({
        let send = send.clone();
        move |_| send(Msg::Changed)
    });
    source
        .add_event_listener_with_callback("changed", on_changed.as_ref().unchecked_ref())
        .ok()?;
    on_changed.forget();

    let on_open = Closure::<dyn FnMut(web_sys::Event)>::new({
        let send = send.clone();
        move |_| send(Msg::Live(true))
    });
    source.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();

    let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| send(Msg::Live(false)));
    source.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();

    Some(source)
}
