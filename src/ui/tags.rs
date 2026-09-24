//! Tag manager popup (`Shift+T` or the top-panel tag button): list every tag
//! with its task count; create, rename, and delete tags.

use crate::database::TagManagement;
use crate::ui::app::{DB, FastTask, UpdateMessage};
use crate::ui::theme::{colors, icons};
use crate::ui::widgets::common;

/// What the popup is doing right now.
#[derive(Default, Debug, Clone, PartialEq)]
pub enum TagUiMode {
    #[default]
    Browse,
    /// Typing a new tag's name.
    New(String),
    /// Renaming `from`; `buf` is the draft name.
    Rename { from: String, buf: String },
    /// Waiting for y / Enter to delete this tag.
    ConfirmDelete(String),
}

#[derive(Default)]
pub struct TagUi {
    pub open: bool,
    pub cursor: usize,
    pub mode: TagUiMode,
    /// `(tag, task count)`, refreshed from the DB on open and after each change.
    pub rows: Vec<(String, usize)>,
    /// Focus the name field on the frame after it appears.
    focus_input: bool,
}

impl TagUi {
    pub fn open(&mut self, tx: std::sync::mpsc::Sender<UpdateMessage>) {
        self.open = true;
        self.mode = TagUiMode::Browse;
        load_usage(tx);
    }

    fn selected(&self) -> Option<&(String, usize)> {
        self.rows.get(self.cursor)
    }
}

fn load_usage(tx: std::sync::mpsc::Sender<UpdateMessage>) {
    crate::ui::bg::spawn(move || match DB.tag_usage() {
        Ok(rows) => {
            tx.send(UpdateMessage::TagUsage(rows)).ok();
        }
        Err(e) => {
            tx.send(UpdateMessage::Error(e)).ok();
        }
    });
}

/// Run a tag mutation in the background, then reload usage, the completion list,
/// and the task list (tags on visible tasks may have changed).
fn run_op(
    tx: std::sync::mpsc::Sender<UpdateMessage>,
    op: impl FnOnce() -> anyhow::Result<usize> + Send + 'static,
) {
    crate::ui::bg::spawn(move || {
        if let Err(e) = op() {
            tx.send(UpdateMessage::Error(e)).ok();
        }
        if let Ok(rows) = DB.tag_usage() {
            tx.send(UpdateMessage::TagUsage(rows)).ok();
        }
        if let Ok(tags) = DB.all_tags() {
            tx.send(UpdateMessage::KnownTags(tags)).ok();
        }
        tx.send(UpdateMessage::Refresh).ok();
    });
}

/// Draw the popup and handle its keys. Call before the app's other keybinds; the
/// caller swallows remaining key events while the popup is open.
pub fn tag_manager(ui: &mut egui::Ui, app: &mut FastTask) {
    use egui::{Key, Modifiers};
    let tx = app.backend_manager.tx.clone();
    let t = &mut app.tag_ui;
    let n = t.rows.len();
    if n > 0 && t.cursor >= n {
        t.cursor = n - 1;
    }

    // --- keys ---
    let mut close = false;
    match t.mode.clone() {
        TagUiMode::Browse => {
            let (down, up, new, rename, delete, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::J)
                        || i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::K)
                        || i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::O)
                        || i.consume_key(Modifiers::NONE, Key::A),
                    i.consume_key(Modifiers::NONE, Key::E)
                        || i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::D)
                        || i.consume_key(Modifiers::NONE, Key::X),
                    i.consume_key(Modifiers::NONE, Key::Escape)
                        || i.consume_key(Modifiers::NONE, Key::Q)
                        || i.consume_key(Modifiers::SHIFT, Key::T),
                )
            });
            if down && n > 0 {
                t.cursor = (t.cursor + 1).min(n - 1);
            }
            if up {
                t.cursor = t.cursor.saturating_sub(1);
            }
            if new {
                t.mode = TagUiMode::New(String::new());
                t.focus_input = true;
            }
            if rename && let Some((tag, _)) = t.selected() {
                t.mode = TagUiMode::Rename {
                    from: tag.clone(),
                    buf: tag.clone(),
                };
                t.focus_input = true;
            }
            if delete && let Some((tag, _)) = t.selected() {
                t.mode = TagUiMode::ConfirmDelete(tag.clone());
            }
            close = esc;
        }
        TagUiMode::ConfirmDelete(tag) => {
            let (yes, no) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::Y)
                        || i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::N)
                        || i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            if yes {
                run_op(tx.clone(), move || DB.delete_tag(&tag));
                t.mode = TagUiMode::Browse;
            } else if no {
                t.mode = TagUiMode::Browse;
            }
        }
        // Name entry: Enter / Esc are handled after the TextEdit below.
        TagUiMode::New(_) | TagUiMode::Rename { .. } => {}
    }
    if close {
        t.open = false;
        t.mode = TagUiMode::Browse;
        return;
    }

    // --- draw ---
    let mut clicked_row: Option<usize> = None;
    let mut click_new = false;
    let mut click_rename: Option<usize> = None;
    let mut click_delete: Option<usize> = None;
    let mut click_close = false;
    let mut submit_name = false;
    let mut cancel_name = false;
    let mut confirm_delete = false;
    let mut cancel_delete = false;

    egui::Modal::new(egui::Id::new("tag_manager")).show(ui.ctx(), |ui| {
        ui.set_min_width(300.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{}  Tags", icons::TAG))
                    .color(colors::LAVENDER)
                    .size(14.0)
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if common::secondary_button(ui, icons::DISCARD)
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    click_close = true;
                }
                if common::secondary_button(ui, format!("{}  New", icons::NEW))
                    .on_hover_text("New tag (o)")
                    .clicked()
                {
                    click_new = true;
                }
            });
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt("tag_manager_rows")
            .max_height(320.0)
            .show(ui, |ui| {
                if t.rows.is_empty() {
                    ui.label(
                        egui::RichText::new("No tags yet — press o to create one.")
                            .color(colors::OVERLAY1)
                            .italics()
                            .size(12.0),
                    );
                }
                for (idx, (tag, count)) in t.rows.iter().enumerate() {
                    let selected = idx == t.cursor && matches!(t.mode, TagUiMode::Browse);
                    let fill = if selected {
                        colors::SURFACE1
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    let row = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(3))
                        .inner_margin(egui::Margin::symmetric(6, 2))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.horizontal(|ui| {
                                // Inline rename field replaces the label for that row.
                                if let TagUiMode::Rename { from, buf } = &mut t.mode
                                    && from == tag
                                {
                                    name_field(
                                        ui,
                                        buf,
                                        &mut t.focus_input,
                                        &mut submit_name,
                                        &mut cancel_name,
                                    );
                                    return;
                                }
                                ui.label(egui::RichText::new(tag).color(colors::TEAL).size(13.0));
                                ui.label(
                                    egui::RichText::new(match count {
                                        1 => "1 task".to_string(),
                                        n => format!("{n} tasks"),
                                    })
                                    .color(colors::OVERLAY1)
                                    .size(11.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if common::secondary_button(ui, icons::DELETE)
                                            .on_hover_text("Delete tag (d)")
                                            .clicked()
                                        {
                                            click_delete = Some(idx);
                                        }
                                        if common::secondary_button(ui, icons::MODE_INSERT)
                                            .on_hover_text("Rename tag (e)")
                                            .clicked()
                                        {
                                            click_rename = Some(idx);
                                        }
                                    },
                                );
                            });
                        });
                    if selected {
                        row.response.scroll_to_me(None);
                    }
                    if row.response.interact(egui::Sense::CLICK).clicked() {
                        clicked_row = Some(idx);
                    }
                }
            });

        match &mut t.mode {
            TagUiMode::New(buf) => {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("New tag")
                            .color(colors::SUBTEXT0)
                            .size(11.0),
                    );
                    name_field(
                        ui,
                        buf,
                        &mut t.focus_input,
                        &mut submit_name,
                        &mut cancel_name,
                    );
                });
            }
            TagUiMode::ConfirmDelete(tag) => {
                let count = t.rows.iter().find(|(r, _)| r == tag).map_or(0, |(_, c)| *c);
                ui.separator();
                ui.label(
                    egui::RichText::new(format!(
                        "Delete \"{tag}\"? It will be removed from {count} task{}.",
                        if count == 1 { "" } else { "s" }
                    ))
                    .color(colors::YELLOW)
                    .size(12.0),
                );
                ui.horizontal(|ui| {
                    if common::danger_button(ui, "Delete (y)").clicked() {
                        confirm_delete = true;
                    }
                    if common::secondary_button(ui, "Cancel (n)").clicked() {
                        cancel_delete = true;
                    }
                });
            }
            _ => {}
        }

        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(match t.mode {
                TagUiMode::Browse => "j/k move · o new · e rename · d delete · Esc close",
                TagUiMode::New(_) | TagUiMode::Rename { .. } => "Enter save · Esc cancel",
                TagUiMode::ConfirmDelete(_) => "y / Enter delete · n / Esc cancel",
            })
            .color(colors::OVERLAY1)
            .size(10.0),
        );
    });

    // --- apply mouse actions + name-field results ---
    if let Some(i) = clicked_row {
        t.cursor = i;
    }
    if click_close {
        t.open = false;
        t.mode = TagUiMode::Browse;
        return;
    }
    if click_new {
        t.mode = TagUiMode::New(String::new());
        t.focus_input = true;
    }
    if let Some(i) = click_rename
        && let Some((tag, _)) = t.rows.get(i)
    {
        t.cursor = i;
        t.mode = TagUiMode::Rename {
            from: tag.clone(),
            buf: tag.clone(),
        };
        t.focus_input = true;
    }
    if let Some(i) = click_delete
        && let Some((tag, _)) = t.rows.get(i)
    {
        t.cursor = i;
        t.mode = TagUiMode::ConfirmDelete(tag.clone());
    }
    if confirm_delete && let TagUiMode::ConfirmDelete(tag) = t.mode.clone() {
        run_op(tx.clone(), move || DB.delete_tag(&tag));
        t.mode = TagUiMode::Browse;
    }
    if cancel_delete || cancel_name {
        t.mode = TagUiMode::Browse;
    }
    if submit_name {
        match t.mode.clone() {
            TagUiMode::New(buf) => {
                let name = buf.trim().to_lowercase();
                if !name.is_empty() {
                    run_op(tx, move || DB.upsert_tags(&[name]).map(|_| 0));
                }
            }
            TagUiMode::Rename { from, buf } if !buf.trim().is_empty() => {
                run_op(tx, move || DB.rename_tag(&from, &buf));
            }
            _ => {}
        }
        t.mode = TagUiMode::Browse;
    }
}

/// Single-line name entry: Enter submits, Esc cancels. Takes focus when first shown.
fn name_field(
    ui: &mut egui::Ui,
    buf: &mut String,
    focus: &mut bool,
    submit: &mut bool,
    cancel: &mut bool,
) {
    let resp = ui.add(
        egui::TextEdit::singleline(buf)
            .id(egui::Id::new("tag_manager_name"))
            .desired_width(180.0)
            .hint_text("tag name"),
    );
    if *focus {
        resp.request_focus();
        *focus = false;
    }
    let (enter, esc) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if enter && (resp.has_focus() || resp.lost_focus()) {
        *submit = true;
    }
    if esc {
        *cancel = true;
    }
}
