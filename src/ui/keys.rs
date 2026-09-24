use egui::{Key, Modifiers};

use crate::ui::app::{DB, Mode, UpdateMessage, WindowState};

/// Returns true when the user pressed `?` to toggle the help popup.
pub fn toggle_help(ui: &egui::Ui) -> bool {
    ui.input(|i| i.key_pressed(egui::Key::Questionmark))
}

/// Returns true when the user pressed `Shift+A` to toggle always-on-top.
pub fn toggle_always_on_top(ui: &egui::Ui) -> bool {
    ui.input(|i| i.modifiers.shift && i.key_pressed(egui::Key::A))
}

/// Returns the new `WindowState` triggered by a global nav key, if any.
///
/// - `t` jumps to Tasks; `Esc` from any other pane returns to Tasks.
/// - `Shift+H` / `Shift+L` step left / right through Projects ↔ Tasks ↔ Info
///   (no wrap-around).
///
/// Esc-to-Projects from Tasks lives in `normal_mode_keybinds`, since it only
/// fires once the filter and selection are already clear. A key that causes a
/// transition is consumed so the destination pane's keybinds, which run later
/// in the same frame, don't act on it too (e.g. Esc bouncing Projects → Tasks
/// → Projects).
pub fn set_window_state(ui: &egui::Ui, mode: &Mode, window: &WindowState) -> Option<WindowState> {
    if !matches!(mode, Mode::Normal) {
        return None;
    }
    let (left, right) = pane_neighbors(window);
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::T) {
            Some(WindowState::Tasks)
        } else if i.consume_key(Modifiers::SHIFT, Key::H) {
            left
        } else if i.consume_key(Modifiers::SHIFT, Key::L) {
            right
        } else if !matches!(window, WindowState::Tasks)
            && i.consume_key(Modifiers::NONE, Key::Escape)
        {
            Some(WindowState::Tasks)
        } else {
            None
        }
    })
}

/// The panes to the left and right of `window` in Projects ↔ Tasks ↔ Info order.
fn pane_neighbors(window: &WindowState) -> (Option<WindowState>, Option<WindowState>) {
    match window {
        WindowState::Projects => (None, Some(WindowState::Tasks)),
        WindowState::Tasks => (Some(WindowState::Projects), Some(WindowState::Info)),
        WindowState::Info => (Some(WindowState::Tasks), None),
    }
}

pub fn set_mode(ui: &egui::Ui, window: &WindowState, mode: &Mode) -> Option<Mode> {
    // The Info pane handles Esc itself when editing.
    if matches!(window, WindowState::Info) && matches!(mode, Mode::Insert(_)) {
        return None;
    }
    // Leaving Visual/Insert consumes the Esc so Normal-mode keybinds running later
    // this frame don't also treat it as "Esc with nothing to clear" → Projects.
    if !matches!(mode, Mode::Normal)
        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
    {
        return Some(Mode::Normal);
    }
    None
}

pub fn undo_redo(ui: &egui::Ui, tx: std::sync::mpsc::Sender<UpdateMessage>) -> anyhow::Result<()> {
    let undo_tx = tx.clone();
    // Plain u / r only — Ctrl+U is half-page up in the task list.
    if ui.input(|i| i.key_pressed(Key::U) && !i.modifiers.ctrl) {
        crate::ui::bg::spawn(move || {
            if let Err(e) = DB.undo() {
                let _ = tx.send(UpdateMessage::Error(e));
                return;
            }
            let _ = tx.send(UpdateMessage::Undone);
        });
    }

    if ui.input(|i| i.key_pressed(Key::R) && !i.modifiers.ctrl) {
        crate::ui::bg::spawn(move || {
            if let Err(e) = DB.redo() {
                let _ = undo_tx.send(UpdateMessage::Error(e));
                return;
            }
            let _ = undo_tx.send(UpdateMessage::Redone);
        });
    }
    Ok(())
}
