use egui::{Response, RichText, Ui, WidgetText};

use crate::ui::theme::colors;

/// Section heading — larger text with the accent color.
pub fn heading(ui: &mut Ui, text: impl Into<String>) {
    ui.label(
        RichText::new(text)
            .size(16.0)
            .color(colors::LAVENDER)
            .strong(),
    );
}

/// Muted label for field names / metadata keys.
pub fn field_label(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.label(RichText::new(text).color(colors::SUBTEXT0).size(11.0))
}

/// Primary action button (filled, accent color).
pub fn primary_button(ui: &mut Ui, text: impl ToString) -> Response {
    let label = egui::RichText::new(text.to_string())
        .color(colors::MANTLE)
        .strong();
    ui.add(
        egui::Button::new(label)
            .fill(colors::BLUE)
            .stroke(egui::Stroke::NONE),
    )
}

/// Subtle / secondary button (outline only).
pub fn secondary_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let btn = egui::Button::new(text)
        .fill(colors::SURFACE1)
        .stroke(egui::Stroke::new(1.0_f32, colors::SURFACE2));
    ui.add(btn)
}

/// Danger button (red fill, for destructive actions).
pub fn danger_button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let btn = egui::Button::new(text)
        .fill(colors::RED)
        .stroke(egui::Stroke::NONE);
    ui.add(btn)
}

/// Render a small colored status badge.
pub fn status_badge(ui: &mut Ui, status: &crate::database::models::TaskStatus) {
    use crate::database::models::TaskStatus;
    use crate::ui::theme::icons;
    let symbol = match status {
        TaskStatus::NotStarted => icons::STATUS_NOT_STARTED,
        TaskStatus::InProgress => icons::STATUS_IN_PROGRESS,
        TaskStatus::Completed => icons::STATUS_COMPLETED,
        TaskStatus::OnHold => icons::STATUS_ON_HOLD,
    };
    let color = crate::ui::theme::status_color(status);
    ui.label(RichText::new(symbol).color(color));
}
