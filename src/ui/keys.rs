use egui::Key;

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
/// `p` is skipped when already in the Tasks pane — `normal_mode_keybinds` owns it there
/// so it can paste from the clipboard instead of navigating.
pub fn set_window_state(ui: &egui::Ui, mode: &Mode, window: &WindowState) -> Option<WindowState> {
    match mode {
        Mode::Normal if ui.input(|i| i.key_pressed(Key::T)) => Some(WindowState::Tasks),
        Mode::Normal
            if !matches!(window, WindowState::Tasks) && ui.input(|i| i.key_pressed(Key::P)) =>
        {
            Some(WindowState::Projects)
        }
        Mode::Normal if ui.input(|i| i.key_pressed(Key::Escape)) => Some(WindowState::Tasks),
        _ => None,
    }
}

pub fn set_mode(ui: &egui::Ui, window: &WindowState, mode: &Mode) -> Option<Mode> {
    // The Info pane handles Esc itself when editing.
    if matches!(window, WindowState::Info) && matches!(mode, Mode::Insert(_)) {
        return None;
    }
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        return Some(Mode::Normal);
    }
    None
}

pub fn undo_redo(ui: &egui::Ui, tx: std::sync::mpsc::Sender<UpdateMessage>) -> anyhow::Result<()> {
    let undo_tx = tx.clone();
    if ui.input(|i| i.key_pressed(Key::U)) {
        std::thread::spawn(move || {
            if let Err(e) = DB.undo() {
                let _ = tx.send(UpdateMessage::Error(e));
                return;
            }
            let _ = tx.send(UpdateMessage::Undone);
        });
    }

    if ui.input(|i| i.key_pressed(Key::R)) {
        std::thread::spawn(move || {
            if let Err(e) = DB.redo() {
                let _ = undo_tx.send(UpdateMessage::Error(e));
                return;
            }
            let _ = undo_tx.send(UpdateMessage::Redone);
        });
    }
    Ok(())
}
