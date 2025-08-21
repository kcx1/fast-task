use crate::database::Database;
use crate::database::ProjectManagement;
use crate::database::TagManagement;
use crate::database::TaskManagement;
use crate::ui::info::info_state;
use crate::ui::keys::{set_mode, set_window_state, toggle_always_on_top, toggle_help, undo_redo};
use crate::ui::theme;
use std::fmt::Debug;
use std::sync::LazyLock;

use anyhow::Error;
use eframe::egui;
use polodb_core::bson::oid::ObjectId;

use crate::database::database::Db;
use crate::database::{Annotation, ProjectEntry, Task};
use crate::local_share;
use crate::ui::projects::{ProjectManager, project_state};
use crate::ui::tasks::{TaskManager, get_tasks, task_state};
use crate::ui::widgets::errors::{ErrorSeverity, ErrorUi};

pub static DB: LazyLock<Db> = LazyLock::new(|| {
    Db::open().unwrap_or_else(|e| {
        panic!(
            "Failed to open database at {:?}: {}\n\
            Quit all running instances and try again, or delete the file to start fresh.",
            crate::database::database::DATABASE.as_path(),
            e
        )
        // TODO(H7): surface as ErrorUi::Fatal once Db is injectable
    })
});

/// Distinguishes deployment contexts so shared rendering code can gate native-only features.
/// Reserved for items 11 (WASM/local share) and 13 (cloud sync); not yet read by any branch.
#[allow(dead_code)]
pub enum AppType {
    Native,
    Shared,
    Web,
}

pub enum WindowState {
    Projects,
    Tasks,
    Info,
}

pub enum Mode {
    Normal,
    Insert(Option<ObjectId>),
    /// Single-cursor visual — move/operate on one task. Entered with `v`.
    Visual,
}

pub struct FastTask {
    pub project_manager: ProjectManager,
    pub task_manager: TaskManager,
    pub backend_manager: BackendManager,
    pub app_state: AppState,
    /// Reserved for item 11 (WASM/local share) and item 13 (cloud sync) branching.
    #[allow(dead_code)]
    pub app_type: AppType,
    pub local_share: bool,
    pub err_ui: ErrorUi,
    pub known_tags: Vec<String>,
    /// Annotations for the currently selected task.
    pub annotations: Vec<Annotation>,
    /// Which task's annotations are currently loaded; compared each frame to detect cursor moves.
    pub annotation_task_id: Option<polodb_core::bson::oid::ObjectId>,
    /// Draft text for a new annotation being composed.
    pub annotation_buf: String,
}
impl Default for FastTask {
    fn default() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<UpdateMessage>();

        #[cfg(target_arch = "wasm32")]
        let app_type = AppType::Shared;
        #[cfg(not(target_arch = "wasm32"))]
        let app_type = AppType::Native;

        Self {
            app_state: AppState {
                mode: Mode::Normal,
                window_state: WindowState::Tasks,
                pending_delete: None,
                show_detail_pane: false,
                show_help: false,
                always_on_top: false,
                init: true,
                status_msg: None,
            },
            project_manager: ProjectManager {
                ..Default::default()
            },
            task_manager: TaskManager {
                ..Default::default()
            },
            backend_manager: BackendManager {
                tx,
                rx,
                backend: std::sync::Arc::new(DB.clone()),
            },
            err_ui: Default::default(),
            local_share: false,
            known_tags: Vec::new(),
            annotations: Vec::new(),
            annotation_task_id: None,
            annotation_buf: String::new(),
            app_type,
        }
    }
}

pub struct AppState {
    pub mode: Mode,
    pub window_state: WindowState,
    pub pending_delete: Option<ObjectId>,
    pub show_detail_pane: bool,
    pub show_help: bool,
    pub always_on_top: bool,
    pub init: bool,
    /// Transient status-bar message with the instant it was set; clears after 3 s.
    pub status_msg: Option<(String, std::time::Instant)>,
}

#[derive(Default, Clone)]
pub enum EditFocus {
    Header,
    Details,
    #[default]
    None,
}

#[derive(Debug)]
pub enum UpdateMessage {
    Projects(Vec<ProjectEntry>),
    CurrentProject(ProjectEntry),
    Tasks(Vec<Task>),
    KnownTags(Vec<String>),
    Annotations(polodb_core::bson::oid::ObjectId, Vec<Annotation>),
    Error(Error),
    DbTransaction(Box<dyn Send + Debug>),
    Refresh,
    Undone,
    Redone,
}

pub struct BackendManager {
    pub tx: std::sync::mpsc::Sender<UpdateMessage>,
    pub rx: std::sync::mpsc::Receiver<UpdateMessage>,
    /// Swappable task storage backend; defaults to the global PoloDB instance.
    pub backend: std::sync::Arc<dyn TaskManagement + Send + Sync>,
}

fn load_icon() -> Option<std::sync::Arc<egui::IconData>> {
    let svg_bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/svg/icon.svg"));
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &opt).ok()?;
    const SIZE: u32 = 256;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE)?;
    let sx = SIZE as f32 / tree.size().width();
    let sy = SIZE as f32 / tree.size().height();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );
    let rgba = unmultiply_alpha(pixmap.take());
    Some(std::sync::Arc::new(egui::IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }))
}

/// tiny-skia outputs premultiplied RGBA; egui expects straight (unmultiplied) RGBA.
fn unmultiply_alpha(data: Vec<u8>) -> Vec<u8> {
    data.chunks_exact(4)
        .flat_map(|p| {
            let a = p[3];
            if a == 0 {
                [0, 0, 0, 0]
            } else {
                let af = a as f32 / 255.0;
                [
                    (p[0] as f32 / af).round().min(255.0) as u8,
                    (p[1] as f32 / af).round().min(255.0) as u8,
                    (p[2] as f32 / af).round().min(255.0) as u8,
                    a,
                ]
            }
        })
        .collect()
}

pub fn run() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([400.0, 600.0]);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "FastTask",
        options,
        Box::new(|cc| {
            use crate::ui::tasks::SortOrder;
            let mut app = FastTask::default();
            if let Some(s) = cc.storage {
                if s.get_string("show_completed").as_deref() == Some("true") {
                    app.task_manager.show_completed = true;
                }
                if s.get_string("show_detail_pane").as_deref() == Some("true") {
                    app.app_state.show_detail_pane = true;
                }
                if s.get_string("always_on_top").as_deref() == Some("true") {
                    app.app_state.always_on_top = true;
                }
                app.task_manager.sort_order = match s.get_string("sort_order").as_deref() {
                    Some("DueDate") => SortOrder::DueDate,
                    Some("Modified") => SortOrder::Modified,
                    Some("Status") => SortOrder::Status,
                    Some("Tags") => SortOrder::Tags,
                    _ => SortOrder::Free,
                };
            }
            Ok(Box::new(app))
        }),
    )
}

impl eframe::App for FastTask {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        use crate::ui::tasks::SortOrder;
        storage.set_string(
            "show_completed",
            self.task_manager.show_completed.to_string(),
        );
        storage.set_string(
            "show_detail_pane",
            self.app_state.show_detail_pane.to_string(),
        );
        storage.set_string("always_on_top", self.app_state.always_on_top.to_string());
        let sort = match self.task_manager.sort_order {
            SortOrder::Free => "Free",
            SortOrder::DueDate => "DueDate",
            SortOrder::Modified => "Modified",
            SortOrder::Status => "Status",
            SortOrder::Tags => "Tags",
        };
        storage.set_string("sort_order", sort.to_string());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Poll for background thread results at 50 ms; egui repaints immediately on any input.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));

        // Apply theme once on startup
        if self.app_state.init {
            theme::apply(ui.ctx());
        }

        // On app open
        self.init(ui.ctx());

        // Update self from background thread results.
        self.thread_sync();

        // Start the local share server on the false→true edge of `local_share`.
        self.start_local_share();

        let mut dropdown_selected: Option<usize> = None;

        egui::Panel::top("Top Panel").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let current_project = match &self.project_manager.current() {
                    ProjectEntry::All => "All".to_string(),
                    ProjectEntry::None => "None".to_string(),
                    ProjectEntry::Project(p) => p.name.clone(),
                };

                egui::ComboBox::new(egui::Id::new("ProjectDropdown"), "")
                    .selected_text(&current_project)
                    .show_ui(ui, |ui| {
                        for (idx, project) in self.project_manager.projects.iter().enumerate() {
                            let selected = idx == self.project_manager.current_project;
                            if ui.selectable_label(selected, project.to_string()).clicked() {
                                dropdown_selected = Some(idx);
                            }
                        }
                    });
            });
        });

        if let Some(idx) = dropdown_selected {
            self.select_project(idx);
        }

        // Status bar — always visible
        {
            use crate::ui::theme::colors;
            egui::Panel::bottom("Status Bar").show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    use crate::ui::theme::icons;
                    let (icon, label, color, tip) = match &self.app_state.mode {
                        Mode::Normal => (
                            icons::MODE_NORMAL,
                            "NORMAL",
                            colors::GREEN,
                            "Normal mode — keyboard navigation",
                        ),
                        Mode::Insert(_) => (
                            icons::MODE_INSERT,
                            "INSERT",
                            colors::BLUE,
                            "Insert mode — editing task",
                        ),
                        Mode::Visual => (
                            icons::MODE_VISUAL,
                            "VISUAL",
                            colors::MAUVE,
                            "Visual mode — reordering task (Esc to exit)",
                        ),
                    };
                    ui.label(
                        egui::RichText::new(format!("{} {}", icon, label))
                            .color(color)
                            .strong()
                            .size(11.0),
                    )
                    .on_hover_text(tip);
                    ui.separator();
                    let project_name = match self.project_manager.current() {
                        ProjectEntry::All => "All".to_string(),
                        ProjectEntry::None => "None".to_string(),
                        ProjectEntry::Project(p) => p.name.clone(),
                    };
                    ui.label(
                        egui::RichText::new(&project_name)
                            .color(colors::SUBTEXT0)
                            .size(11.0),
                    );

                    // Show Tab-selection count when tasks are selected
                    let sel = self.task_manager.selected_tasks.len();
                    if sel > 0 {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("{sel} selected"))
                                .color(colors::YELLOW)
                                .size(11.0),
                        );
                    }

                    if self.task_manager.filter_open {
                        ui.separator();
                        ui.label(egui::RichText::new("/").color(colors::OVERLAY1).size(11.0));
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.task_manager.filter_query)
                                .id(egui::Id::new("task_filter_input"))
                                .desired_width(140.0)
                                .hint_text("filter…")
                                .font(egui::TextStyle::Small),
                        );
                        if self.task_manager.filter_just_opened {
                            resp.request_focus();
                            self.task_manager.filter_just_opened = false;
                        }
                    } else if !self.task_manager.filter_query.is_empty() {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!(
                                "filter: {}",
                                self.task_manager.filter_query
                            ))
                            .color(colors::PEACH)
                            .size(11.0),
                        );
                    }

                    if self.task_manager.show_completed {
                        ui.separator();
                        ui.label(
                            egui::RichText::new("+ completed")
                                .color(colors::GREEN)
                                .size(11.0),
                        )
                        .on_hover_text("Showing completed tasks (Shift+C to toggle)");
                    }

                    {
                        use crate::ui::tasks::SortOrder;
                        if self.task_manager.sort_order != SortOrder::Free {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(format!(
                                    "sort: {}",
                                    self.task_manager.sort_order.label()
                                ))
                                .color(colors::TEAL)
                                .size(11.0),
                            )
                            .on_hover_text("Active sort order (Shift+S to change)");
                        }
                    }

                    if self.app_state.always_on_top {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("{} pinned", icons::PIN))
                                .color(colors::MAUVE)
                                .size(11.0),
                        )
                        .on_hover_text("Window pinned above all others (Shift+A to toggle)");
                    }

                    // Transient status message (Undone / Redone / Deleted hint)
                    let msg_expired = if let Some((msg, since)) = &self.app_state.status_msg {
                        if since.elapsed() < std::time::Duration::from_secs(3) {
                            ui.separator();
                            ui.label(
                                egui::RichText::new(msg.as_str())
                                    .color(colors::TEAL)
                                    .size(11.0),
                            );
                            false
                        } else {
                            true
                        }
                    } else {
                        false
                    };
                    if msg_expired {
                        self.app_state.status_msg = None;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("? for help")
                                .color(colors::OVERLAY0)
                                .size(11.0),
                        );
                    });
                });
            });
        }

        // Gate all global keybinds while any text widget has keyboard focus so that typing
        // in e.g. the annotation input doesn't fire navigation or undo/redo actions.
        if !ui.ctx().egui_wants_keyboard_input() {
            if toggle_help(ui) {
                self.app_state.show_help = !self.app_state.show_help;
            }
            if toggle_always_on_top(ui) {
                self.app_state.always_on_top = !self.app_state.always_on_top;
                let level = if self.app_state.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                };
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
            }

            // Keybinds to change state and mode
            if let Some(ws) =
                set_window_state(ui, &self.app_state.mode, &self.app_state.window_state)
            {
                self.app_state.window_state = ws;
            }
            if let Some(mode) = set_mode(ui, &self.app_state.window_state, &self.app_state.mode) {
                self.app_state.mode = mode;
            }
            if let Err(e) = undo_redo(ui, self.backend_manager.tx.clone()) {
                ErrorUi::push(&mut self.err_ui, e, ErrorSeverity::NonFatal);
            }
        }

        // Handle the different window states
        match &self.app_state.window_state {
            WindowState::Projects => project_state(ui, self),
            WindowState::Info => info_state(ui, self),
            WindowState::Tasks => task_state(ui, self),
        };

        // Keybinding help popup
        if self.app_state.show_help {
            show_help_popup(
                ui,
                &self.app_state.mode,
                &self.app_state.window_state,
                &mut self.app_state.show_help,
            );
        }

        // Show errors every frame
        self.err_ui.show(ui);
    }
}

impl FastTask {
    fn refresh_tasks(&mut self) {
        get_tasks(
            self.backend_manager.backend.clone(),
            self.project_manager.current().clone(),
            self.task_manager.show_completed,
            self.backend_manager.tx.clone(),
        );
    }

    fn refresh_projects(&self) {
        let tx = self.backend_manager.tx.clone();
        std::thread::spawn(move || match DB.all_projects() {
            Ok(real) => {
                let projects = crate::ui::projects::assemble_project_list(real);
                let _ = tx.send(UpdateMessage::Projects(projects));
            }
            Err(e) => {
                let _ = tx.send(UpdateMessage::Error(e));
            }
        });
    }

    /// Selects a project by index: updates current/hovered, persists the choice, and kicks off
    /// a task refresh. Does not change `window_state` (use `confirm_project` in projects.rs for that).
    pub fn select_project(&mut self, idx: usize) {
        self.project_manager.current_project = idx;
        self.project_manager.hovered_project = idx;
        if let Some(project) = self.project_manager.projects.get(idx) {
            let project = project.clone();
            if let Err(e) = DB.save_current_project(project.clone()) {
                ErrorUi::push(&mut self.err_ui, e, ErrorSeverity::NonFatal);
            }
            get_tasks(
                self.backend_manager.backend.clone(),
                project,
                self.task_manager.show_completed,
                self.backend_manager.tx.clone(),
            );
        }
    }

    fn refresh_tags(&self) {
        let tx = self.backend_manager.tx.clone();
        std::thread::spawn(move || {
            if let Ok(tags) = DB.all_tags() {
                let _ = tx.send(UpdateMessage::KnownTags(tags));
            }
        });
    }

    fn init(&mut self, ctx: &egui::Context) {
        if self.app_state.init {
            self.app_state.init = false;

            if self.app_state.always_on_top {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::AlwaysOnTop,
                ));
            }

            let tx = self.backend_manager.tx.clone();
            std::thread::spawn(move || {
                let entry = DB.get_recent_project().unwrap_or(ProjectEntry::All);
                let _ = tx.send(UpdateMessage::CurrentProject(entry));
            });

            self.refresh_tags();
            self.refresh_tasks();
        }
    }

    fn start_local_share(&self) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        if self.local_share {
            runtime.spawn(async {
                local_share::server::start_server();
            });
        }
    }

    fn thread_sync(&mut self) {
        while let Ok(message) = self.backend_manager.rx.try_recv() {
            match message {
                UpdateMessage::Refresh => {
                    self.refresh_tasks();
                    self.refresh_projects();
                }
                UpdateMessage::Projects(prj) => {
                    let len = prj.len();
                    self.project_manager.projects = prj;
                    if self.project_manager.current_project >= len {
                        self.project_manager.current_project = 0;
                    }
                    self.refresh_tasks();
                }
                UpdateMessage::CurrentProject(prj) => {
                    if let Some(idx) = self.project_manager.index_of(&prj) {
                        self.project_manager.current_project = idx;
                        self.project_manager.hovered_project = idx;
                    }
                    self.refresh_tasks();
                }
                UpdateMessage::Tasks(tsk) => {
                    self.task_manager.tasks = tsk;
                    self.task_manager.filter_query.clear();
                    self.task_manager.filter_open = false;
                    // Cursor clamping is handled each frame in task_state
                }
                UpdateMessage::KnownTags(tags) => {
                    self.known_tags = tags;
                }
                UpdateMessage::Annotations(task_id, anns) => {
                    if self.annotation_task_id == Some(task_id) {
                        self.annotations = anns;
                    }
                }
                UpdateMessage::DbTransaction(_res) => {
                    self.task_manager.writer.flush();
                    self.refresh_tasks();
                    self.refresh_tags();
                }
                UpdateMessage::Error(e) => {
                    ErrorUi::push(&mut self.err_ui, e, ErrorSeverity::NonFatal);
                }
                UpdateMessage::Undone => {
                    self.app_state.status_msg =
                        Some(("Undone".to_string(), std::time::Instant::now()));
                    self.refresh_tasks();
                    self.refresh_projects();
                }
                UpdateMessage::Redone => {
                    self.app_state.status_msg =
                        Some(("Redone".to_string(), std::time::Instant::now()));
                    self.refresh_tasks();
                    self.refresh_projects();
                }
            }
        }
    }
}

fn show_help_popup(ui: &mut egui::Ui, mode: &Mode, window: &WindowState, show: &mut bool) {
    use crate::ui::theme::colors;

    let bindings: &[(&str, &str)] = match (mode, window) {
        (Mode::Normal, WindowState::Projects) => &[
            ("Projects pane", ""),
            ("j / k", "Move cursor down / up"),
            ("Enter", "Select project, go to Tasks"),
            ("o", "New project"),
            ("e", "Rename hovered project"),
            ("d", "Delete hovered project"),
            ("", ""),
            ("View", ""),
            ("t", "Go to Tasks pane"),
            ("?", "Toggle this help"),
        ],
        (Mode::Normal, _) => &[
            ("Navigation", ""),
            ("j / k", "Move cursor down / up"),
            ("", ""),
            ("Tasks", ""),
            ("o / O", "New task below / above"),
            ("i / e", "Edit selected task"),
            ("y", "Yank (copy) task"),
            ("p", "Paste yanked task below cursor (or go to Projects)"),
            ("d", "Mark complete (remove from view)"),
            ("Shift+D", "Hard delete task"),
            ("s", "Set status (popup picker)"),
            ("Tab", "Toggle task in selection set (Esc to clear)"),
            ("", ""),
            ("Modes", ""),
            ("v", "Visual mode (reorder single task)"),
            ("Shift+S", "Sort tasks (popup picker)"),
            ("", ""),
            ("View", ""),
            ("Shift+K", "Toggle detail pane"),
            ("Shift+C", "Show / hide completed tasks"),
            ("Shift+A", "Toggle always-on-top"),
            ("t", "Go to Tasks pane"),
            ("p (no clipboard)", "Go to Projects pane"),
            ("u / r", "Undo / Redo"),
            ("?", "Toggle this help"),
            ("", ""),
            ("Filter", ""),
            ("/", "Open filter bar"),
            ("Enter", "Confirm filter (bar hides, list stays narrow)"),
            ("Esc", "Clear filter / selection and close bar"),
        ],
        (Mode::Insert(_), _) => &[
            ("Task Editor", ""),
            ("Enter", "Save and return to Tasks"),
            ("Esc", "Discard and return to Tasks"),
        ],
        (Mode::Visual, _) => &[
            ("Visual Mode  (single task)", ""),
            ("j / k", "Move cursor"),
            ("Shift+J", "Move task down in list"),
            ("Shift+K", "Move task up in list"),
            ("d / Shift+D", "Complete / delete task"),
            ("s", "Set status (popup picker)"),
            ("i / e", "Edit task"),
            ("Esc", "Return to Normal"),
        ],
    };

    egui::Modal::new(egui::Id::new("help_popup")).show(ui.ctx(), |ui| {
        ui.set_min_width(340.0);
        ui.label(
            egui::RichText::new("Keybindings  (Esc to close)")
                .color(colors::LAVENDER)
                .size(15.0)
                .strong(),
        );
        ui.separator();
        ui.add_space(4.0);

        let max_h = ui.ctx().viewport_rect().height() - 120.0;
        egui::ScrollArea::vertical()
            .max_height(max_h.max(200.0))
            .show(ui, |ui| {
                egui::Grid::new("help_grid")
                    .num_columns(2)
                    .spacing([16.0, 3.0])
                    .show(ui, |ui| {
                        for (key, desc) in bindings {
                            if key.is_empty() {
                                ui.end_row();
                                continue;
                            }
                            if desc.is_empty() {
                                // Section header
                                ui.label(
                                    egui::RichText::new(*key)
                                        .color(colors::BLUE)
                                        .strong()
                                        .size(11.0),
                                );
                                ui.label("");
                            } else {
                                ui.label(
                                    egui::RichText::new(*key)
                                        .color(colors::YELLOW)
                                        .size(12.0)
                                        .monospace(),
                                );
                                ui.label(egui::RichText::new(*desc).color(colors::TEXT).size(11.0));
                            }
                            ui.end_row();
                        }
                    });
            });

        ui.add_space(6.0);
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *show = false;
        }
    });
}
