use anyhow::Error;
use eframe::egui;

use crate::ui::theme::colors;
use crate::ui::widgets::common;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Fatal,
    NonFatal,
}

/// One captured error, split into a clean user-facing line and the full
/// technical chain. Errors here are all `anyhow` with human-authored context
/// strings (`.context("Failed to open DB")` etc.), so there are no concrete
/// error *types* to categorize — instead we lean on anyhow's own layering:
/// `Display` (`{err}`) is the top-level message users should read, while the
/// `{err:#}` chain carries the internal context (paths, method names) that only
/// helps when debugging. We show the former and tuck the latter behind
/// "Details".
struct ErrorEntry {
    /// Top-level, user-facing message — no internal chain.
    friendly: String,
    /// Full anyhow chain, revealed only on demand.
    details: String,
    /// Whether the "Details" disclosure is expanded (non-fatal banners).
    details_open: bool,
}

impl ErrorEntry {
    fn new(err: &Error) -> Self {
        Self {
            friendly: format!("{err}"),
            details: format!("{err:#}"),
            details_open: false,
        }
    }

    /// True when the full chain says more than the friendly line already shows,
    /// i.e. there is something worth putting behind "Details".
    fn has_extra_details(&self) -> bool {
        self.details != self.friendly
    }
}

#[derive(Default)]
pub struct ErrorUi {
    /// Fatal errors are shown one at a time; user must dismiss before continuing.
    fatal: Vec<ErrorEntry>,
    /// Non-fatal errors queue up; each can be dismissed independently.
    non_fatal: Vec<ErrorEntry>,
}

impl ErrorUi {
    pub fn push(&mut self, err: Error, severity: ErrorSeverity) -> &mut Self {
        let entry = ErrorEntry::new(&err);
        match severity {
            ErrorSeverity::Fatal => self.fatal.push(entry),
            ErrorSeverity::NonFatal => self.non_fatal.push(entry),
        }
        self
    }

    /// Call every frame inside the egui update loop.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.show_fatal(&ctx);
        self.show_non_fatal(&ctx);
    }

    fn show_fatal(&mut self, ctx: &egui::Context) {
        if self.fatal.is_empty() {
            return;
        }
        let friendly = self.fatal[0].friendly.clone();
        let details = self.fatal[0].details.clone();
        let has_details = self.fatal[0].has_extra_details();
        let mut exit = false;
        let mut dismiss = false;

        egui::Modal::new(egui::Id::new("fatal_error_modal")).show(ctx, |ui| {
            ui.set_min_width(320.0);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Fatal Error")
                        .color(colors::RED)
                        .size(16.0)
                        .strong(),
                );
                ui.separator();
                ui.add_space(4.0);
                // Friendly headline first; the raw chain sits below in a
                // collapsible so it doesn't hide crash detail but no longer
                // greets the user as a wall of internal context.
                ui.label(
                    egui::RichText::new(&friendly)
                        .color(colors::TEXT)
                        .size(13.0),
                );
                if has_details {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("Details")
                        .id_salt("fatal_error_details")
                        .default_open(true)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(200.0)
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(&details)
                                            .color(colors::SUBTEXT0)
                                            .size(11.0),
                                    );
                                });
                        });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if common::danger_button(ui, "Exit App").clicked() {
                        exit = true;
                    }
                    if common::secondary_button(ui, "Dismiss").clicked() {
                        dismiss = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter))
                {
                    dismiss = true;
                }
                if self.fatal.len() > 1 {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("+{} more", self.fatal.len() - 1))
                            .color(colors::SUBTEXT0)
                            .size(11.0),
                    );
                }
            });
        });

        if exit {
            std::process::exit(1);
        }
        if dismiss {
            self.fatal.remove(0);
        }
    }

    fn show_non_fatal(&mut self, ctx: &egui::Context) {
        if self.non_fatal.is_empty() {
            return;
        }

        // Show each non-fatal as a dismissible banner stacked near the top
        let mut to_remove = vec![];
        let mut to_toggle = vec![];
        for (i, entry) in self.non_fatal.iter().enumerate() {
            let id = egui::Id::new(("non_fatal_banner", i));
            let y_offset = 8.0 + i as f32 * 52.0;

            egui::Window::new("##nfe")
                .id(id)
                .collapsible(false)
                .resizable(false)
                .title_bar(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, y_offset])
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(colors::SURFACE0)
                        .stroke(egui::Stroke::new(1.0_f32, colors::YELLOW))
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::same(8_i8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("⚠").color(colors::YELLOW));
                                ui.label(
                                    egui::RichText::new(entry.friendly.as_str())
                                        .color(colors::TEXT)
                                        .size(12.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if common::secondary_button(ui, "✕")
                                            .on_hover_text("Dismiss")
                                            .clicked()
                                        {
                                            to_remove.push(i);
                                        }
                                        if entry.has_extra_details() {
                                            let label = if entry.details_open {
                                                "Hide"
                                            } else {
                                                "Details"
                                            };
                                            if common::secondary_button(ui, label)
                                                .on_hover_text("Show technical details")
                                                .clicked()
                                            {
                                                to_toggle.push(i);
                                            }
                                        }
                                    },
                                );
                            });
                            if entry.details_open {
                                ui.add_space(4.0);
                                egui::ScrollArea::vertical()
                                    .id_salt(("non_fatal_details", i))
                                    .max_height(120.0)
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(entry.details.as_str())
                                                .color(colors::SUBTEXT0)
                                                .size(11.0),
                                        );
                                    });
                            }
                        });
                });
        }

        for i in to_toggle {
            if let Some(entry) = self.non_fatal.get_mut(i) {
                entry.details_open = !entry.details_open;
            }
        }
        for i in to_remove.into_iter().rev() {
            self.non_fatal.remove(i);
        }
    }
}
