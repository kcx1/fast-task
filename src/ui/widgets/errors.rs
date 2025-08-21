use anyhow::Error;
use eframe::egui;

use crate::ui::theme::colors;
use crate::ui::widgets::common;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    Fatal,
    NonFatal,
}

#[derive(Default)]
pub struct ErrorUi {
    /// Fatal errors are shown one at a time; user must dismiss before continuing.
    fatal: Vec<String>,
    /// Non-fatal errors queue up; each can be dismissed independently.
    non_fatal: Vec<String>,
}

impl ErrorUi {
    pub fn push(&mut self, err: Error, severity: ErrorSeverity) -> &mut Self {
        let msg = match severity {
            // Fatal: full anyhow chain so nothing is hidden when debugging a crash.
            ErrorSeverity::Fatal => format!("{err:#}"),
            // Non-fatal: top-level message only; the chain reads like internal code noise to users.
            ErrorSeverity::NonFatal => format!("{err}"),
        };
        match severity {
            ErrorSeverity::Fatal => self.fatal.push(msg),
            ErrorSeverity::NonFatal => self.non_fatal.push(msg),
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
        let msg = self.fatal[0].clone();
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
                egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&msg).color(colors::TEXT).size(12.0));
                    });
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
        for (i, msg) in self.non_fatal.iter().enumerate() {
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
                        .stroke(egui::Stroke::new(1.0, colors::YELLOW))
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::same(8_i8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("⚠").color(colors::YELLOW));
                                ui.label(
                                    egui::RichText::new(msg.as_str())
                                        .color(colors::TEXT)
                                        .size(12.0),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if common::secondary_button(ui, "✕").clicked() {
                                            to_remove.push(i);
                                        }
                                    },
                                );
                            });
                        });
                });
        }

        for i in to_remove.into_iter().rev() {
            self.non_fatal.remove(i);
        }
    }
}
