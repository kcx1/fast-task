use egui::InnerResponse;
use egui::Key;
use polodb_core::bson::oid::ObjectId;

use crate::database::ProjectEntry;
use crate::database::ProjectManagement;
use crate::database::models::Project;
use crate::ui::app::DB;
use crate::ui::app::EditFocus;
use crate::ui::app::FastTask;
use crate::ui::app::Mode;
use crate::ui::app::UpdateMessage;
use crate::ui::app::WindowState;

/// Wraps a flat list of real projects in the `[All] … [None]` sentinel entries.
pub(crate) fn assemble_project_list(real: Vec<ProjectEntry>) -> Vec<ProjectEntry> {
    let mut projects = vec![ProjectEntry::All];
    projects.extend(real);
    projects.push(ProjectEntry::None);
    projects
}

pub struct ProjectManager {
    pub projects: Vec<ProjectEntry>,
    pub current_project: usize,
    pub hovered_project: usize,
    pub writer: ProjectWriter,
}

impl Default for ProjectManager {
    fn default() -> Self {
        // Load projects fallibly; a DB error produces an empty list rather than a panic.
        // Errors are surfaced later through UpdateMessage::Error once the UI is live.
        let real = DB.all_projects().unwrap_or_default();
        let projects = assemble_project_list(real);

        Self {
            projects,
            current_project: 0,
            hovered_project: 0,
            writer: ProjectWriter::default(),
        }
    }
}

impl ProjectManager {
    pub fn current(&self) -> &ProjectEntry {
        &self.projects[self.current_project]
    }

    pub fn index_of(&self, entry: &ProjectEntry) -> Option<usize> {
        self.projects.iter().position(|p| match (p, entry) {
            (ProjectEntry::All, ProjectEntry::All) => true,
            (ProjectEntry::None, ProjectEntry::None) => true,
            (ProjectEntry::Project(a), ProjectEntry::Project(b)) => a.id == b.id,
            _ => false,
        })
    }

    /// Sync hover to current — called by task_state so returning to Tasks always resets hover.
    pub fn sync_hover(&mut self) {
        self.hovered_project = self.current_project;
    }
}

#[derive(Default, Clone)]
pub struct ProjectWriter {
    pub single_line_buffer: String,
    pub has_focus: EditFocus,
}

impl ProjectWriter {
    pub fn flush(&mut self) {
        self.single_line_buffer.clear();
        self.has_focus = EditFocus::None;
    }
}

pub fn project_state(ui: &mut egui::Ui, app: &mut FastTask) -> InnerResponse<()> {
    egui::CentralPanel::default().show_inside(ui, |ui| {
        // Flush creation buffer when back in Normal mode
        if let Mode::Normal = app.app_state.mode {
            app.project_manager.writer.flush();
        }

        keybinds(ui, app);

        let mut clicked_idx: Option<usize> = None;
        let mut trash_id: Option<ObjectId> = None;

        {
            use crate::ui::theme::{colors, icons};
            egui::ScrollArea::vertical()
                .id_salt("projects_scroll")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let projects_len = app.project_manager.projects.len();
                    for idx in 0..projects_len {
                        let project = &app.project_manager.projects[idx];
                        let is_hovered = idx == app.project_manager.hovered_project;
                        let is_active = idx == app.project_manager.current_project;
                        let is_odd = idx % 2 != 0;

                        let text_color = if is_active {
                            colors::MANTLE
                        } else {
                            colors::TEXT
                        };

                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 26.0),
                            egui::Sense::click(),
                        );

                        if is_active {
                            ui.painter().rect_filled(rect, 3.0, colors::BLUE);
                        } else if is_hovered {
                            ui.painter().rect_filled(rect, 3.0, colors::SURFACE1);
                        } else if is_odd {
                            ui.painter().rect_filled(rect, 0.0, colors::MANTLE);
                        }

                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                        child.horizontal_centered(|ui| {
                            ui.add_space(8.0);
                            ui.label(
                                egui::RichText::new(project.to_string())
                                    .color(text_color)
                                    .size(13.0),
                            );

                            if let ProjectEntry::Project(p) = project {
                                let delete_color = if is_active {
                                    colors::MANTLE
                                } else {
                                    colors::OVERLAY0
                                };
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.add_space(4.0);
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new(icons::DELETE)
                                                        .color(delete_color)
                                                        .size(11.0),
                                                )
                                                .fill(egui::Color32::TRANSPARENT)
                                                .stroke(egui::Stroke::NONE),
                                            )
                                            .on_hover_text("Delete project (d)")
                                            .clicked()
                                        {
                                            trash_id = Some(p.id);
                                        }
                                    },
                                );
                            }
                        });

                        if resp.clicked() {
                            clicked_idx = Some(idx);
                        }
                        if resp.hovered() {
                            app.project_manager.hovered_project = idx;
                        }
                    }
                });
        }

        if let Some(idx) = clicked_idx {
            confirm_project(app, idx);
        }
        if let Some(id) = trash_id {
            app.app_state.pending_delete = Some(id);
        }

        // Inline creation form
        if let Mode::Insert(None) = app.app_state.mode {
            ui.separator();
            let response =
                ui.text_edit_singleline(&mut app.project_manager.writer.single_line_buffer);

            if let EditFocus::None = app.project_manager.writer.has_focus {
                response.request_focus();
                app.project_manager.writer.has_focus = EditFocus::Header;
            }

            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if enter {
                let name = app
                    .project_manager
                    .writer
                    .single_line_buffer
                    .trim()
                    .to_string();
                if !name.is_empty() {
                    project_submit_create(name, app.backend_manager.tx.clone());
                }
                app.project_manager.writer.flush();
                app.app_state.mode = Mode::Normal;
            }
        }

        // Inline rename form
        if let Mode::Insert(Some(edit_id)) = app.app_state.mode {
            ui.separator();
            let response =
                ui.text_edit_singleline(&mut app.project_manager.writer.single_line_buffer);

            if let EditFocus::None = app.project_manager.writer.has_focus {
                response.request_focus();
                app.project_manager.writer.has_focus = EditFocus::Header;
            }

            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if enter {
                let name = app
                    .project_manager
                    .writer
                    .single_line_buffer
                    .trim()
                    .to_string();
                if !name.is_empty()
                    && let Some(ProjectEntry::Project(mut p)) = app
                        .project_manager
                        .projects
                        .iter()
                        .find(|e| matches!(e, ProjectEntry::Project(p) if p.id == edit_id))
                        .cloned()
                {
                    p.name = name;
                    project_submit_rename(p, app.backend_manager.tx.clone());
                }
                app.project_manager.writer.flush();
                app.app_state.mode = Mode::Normal;
            }
        }

        // Deletion confirmation modal
        if let Some(delete_id) = app.app_state.pending_delete {
            let mut confirm = false;
            let mut cancel = false;
            let modal = egui::containers::Modal::new(egui::Id::new("ConfirmDeleteProject"));
            modal.show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.heading("Delete this project?");
                    ui.label("All associated tasks will also be deleted.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Confirm").clicked() {
                            confirm = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                    let (enter, esc) =
                        ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                    if enter {
                        confirm = true;
                    }
                    if esc {
                        cancel = true;
                    }
                });
            });
            if confirm {
                if let ProjectEntry::Project(p) = app.project_manager.current()
                    && p.id == delete_id
                {
                    app.project_manager.current_project = 0;
                    app.project_manager.hovered_project = 0;
                }
                project_submit_delete(delete_id, app.backend_manager.tx.clone());
                app.app_state.pending_delete = None;
            } else if cancel {
                app.app_state.pending_delete = None;
            }
        }
    })
}

fn keybinds(ui: &egui::Ui, app: &mut FastTask) {
    if !matches!(app.app_state.mode, Mode::Normal) {
        return;
    }

    let len = app.project_manager.projects.len();

    let (pressed_o, pressed_e, pressed_d, pressed_j, pressed_k, pressed_enter) = ui.input(|i| {
        (
            i.key_pressed(Key::O),
            i.key_pressed(Key::E),
            i.key_pressed(Key::D),
            i.key_pressed(Key::J),
            i.key_pressed(Key::K),
            i.key_pressed(Key::Enter),
        )
    });

    if pressed_o {
        app.app_state.mode = Mode::Insert(None);
    }
    if pressed_e {
        let hovered = app.project_manager.hovered_project;
        if let Some(ProjectEntry::Project(p)) = app.project_manager.projects.get(hovered) {
            app.project_manager.writer.single_line_buffer = p.name.clone();
            app.project_manager.writer.has_focus = EditFocus::None;
            app.app_state.mode = Mode::Insert(Some(p.id));
        }
    }
    if pressed_d {
        let hovered = app.project_manager.hovered_project;
        if let Some(ProjectEntry::Project(p)) = app.project_manager.projects.get(hovered) {
            app.app_state.pending_delete = Some(p.id);
        }
    }
    if pressed_j {
        app.project_manager.hovered_project =
            (app.project_manager.hovered_project + 1).min(len.saturating_sub(1));
    }
    if pressed_k {
        app.project_manager.hovered_project = app.project_manager.hovered_project.saturating_sub(1);
    }
    if pressed_enter {
        let idx = app.project_manager.hovered_project;
        confirm_project(app, idx);
    }
}

/// Applies a project selection and switches to the Tasks pane.
fn confirm_project(app: &mut FastTask, idx: usize) {
    app.select_project(idx);
    app.app_state.window_state = WindowState::Tasks;
}

fn project_submit_create(name: String, tx: std::sync::mpsc::Sender<UpdateMessage>) {
    std::thread::spawn(move || {
        if let Err(e) = DB.create_project(Project::new(&name, None)) {
            let _ = tx.send(UpdateMessage::Error(e));
            return;
        }
        match DB.all_projects() {
            Ok(real) => {
                let _ = tx.send(UpdateMessage::Projects(assemble_project_list(real)));
            }
            Err(e) => {
                let _ = tx.send(UpdateMessage::Error(e));
            }
        }
    });
}

fn project_submit_delete(id: ObjectId, tx: std::sync::mpsc::Sender<UpdateMessage>) {
    std::thread::spawn(move || {
        if let Err(e) = DB.delete_project(id) {
            let _ = tx.send(UpdateMessage::Error(e));
            return;
        }
        match DB.all_projects() {
            Ok(real) => {
                let _ = tx.send(UpdateMessage::Projects(assemble_project_list(real)));
            }
            Err(e) => {
                let _ = tx.send(UpdateMessage::Error(e));
            }
        }
    });
}

fn project_submit_rename(
    project: crate::database::models::Project,
    tx: std::sync::mpsc::Sender<UpdateMessage>,
) {
    std::thread::spawn(move || {
        if let Err(e) = DB.update_project(project) {
            let _ = tx.send(UpdateMessage::Error(e));
            return;
        }
        match DB.all_projects() {
            Ok(real) => {
                let _ = tx.send(UpdateMessage::Projects(assemble_project_list(real)));
            }
            Err(e) => {
                let _ = tx.send(UpdateMessage::Error(e));
            }
        }
    });
}
