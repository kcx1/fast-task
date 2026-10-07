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
#[cfg(not(target_arch = "wasm32"))]
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

/// Render a small colored status badge. The symbol is icon-only, so a hover
/// tooltip names the status for anyone who doesn't recognize the glyph.
pub fn status_badge(ui: &mut Ui, status: &crate::database::models::TaskStatus) {
    use crate::ui::view::{status_icon, status_name};
    let color = crate::ui::theme::status_color(status);
    ui.label(RichText::new(status_icon(status)).color(color))
        .on_hover_text(status_name(status));
}

/// A one-line text field with an "Add" button after it; true when the text
/// should be added (button, or Enter in the field).
///
/// The row is exactly one line tall and the field takes exactly the width the
/// button leaves. Guessing the button's width instead overflowed the row, and
/// inside a resizable panel that re-sized the panel every frame; laying it out
/// right to left without a fixed height centered it in all the space below.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn add_row(ui: &mut Ui, buf: &mut String, hint_text: &str) -> bool {
    let height = ui
        .spacing()
        .interact_size
        .y
        .max(ui.text_style_height(&egui::TextStyle::Button))
        + 2.0 * ui.spacing().button_padding.y;
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let clicked = secondary_button(ui, "Add").clicked();
            let resp = ui.add(
                egui::TextEdit::singleline(buf)
                    .hint_text(hint_text)
                    .desired_width(ui.available_width()),
            );
            clicked || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        },
    )
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In a tall resizable side panel (the browser's task list) the row stays
    /// one line high at the top, and the panel keeps its width frame to frame.
    #[test]
    fn add_row_is_one_line_and_does_not_resize_its_panel() {
        let ctx = egui::Context::default();
        let mut buf = String::new();
        let (mut widths, mut rows) = (Vec::new(), Vec::new());
        for _ in 0..30 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(1000.0, 800.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run_ui(input, |ui| {
                let panel = egui::Panel::left("list")
                    .resizable(true)
                    .default_size(360.0)
                    .show_inside(ui, |ui| {
                        let top = ui.cursor().top();
                        add_row(ui, &mut buf, "Add a task…");
                        rows.push((top, ui.cursor().top()));
                    });
                widths.push(panel.response.rect.width());
            });
        }
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "panel width drifts: {widths:?}"
        );
        let (top, after) = *rows.last().unwrap();
        assert!(
            after - top < 40.0,
            "row is {} tall, not one line",
            after - top
        );
    }
}
