//! Syntax-highlighted task details, via `egui_extras`' syntect highlighter
//! (memoized by egui, so these are cheap to call every frame).

use crate::database::models::CodeLanguage;
use egui_extras::syntax_highlighting::{CodeTheme, highlight};

fn theme(ui: &egui::Ui) -> CodeTheme {
    CodeTheme::from_style(ui.style())
}

/// Highlighted layout for `code` as `language`.
pub fn layout_job(ui: &egui::Ui, code: &str, language: CodeLanguage) -> egui::text::LayoutJob {
    highlight(
        ui.ctx(),
        ui.style(),
        &theme(ui),
        code,
        language.syntax_key(),
    )
}

/// Read-only details: highlighted when a language is set, monospace when only
/// `code` is, plain text otherwise. Selectable so snippets can be copied.
pub fn details_view(ui: &mut egui::Ui, details: &str, code: bool, language: Option<CodeLanguage>) {
    match (code, language) {
        (_, Some(lang)) => {
            let job = layout_job(ui, details, lang);
            ui.add(egui::Label::new(job).selectable(true));
        }
        (true, None) => {
            ui.add(egui::Label::new(egui::RichText::new(details).monospace()).selectable(true));
        }
        (false, None) => {
            ui.label(egui::RichText::new(details).size(13.0));
        }
    }
}

/// Multiline editor for details. With a language set, text is highlighted live as
/// you type. Shift+Enter inserts a newline (plain Enter saves the form), matching
/// the plain-text editor.
pub fn details_editor(
    ui: &mut egui::Ui,
    buf: &mut String,
    code: bool,
    language: Option<CodeLanguage>,
) -> egui::Response {
    let newline = egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter);
    let edit = egui::TextEdit::multiline(buf)
        .id(egui::Id::new("details_input"))
        .desired_width(f32::INFINITY)
        .desired_rows(if code { 8 } else { 4 })
        .return_key(newline);
    match (code, language) {
        (_, Some(lang)) => {
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                let mut job = layout_job(ui, text.as_str(), lang);
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            ui.add(edit.code_editor().layouter(&mut layouter))
        }
        (true, None) => ui.add(edit.code_editor()),
        (false, None) => ui.add(edit.hint_text("Additional context…")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_resolves_to_a_syntect_syntax() {
        let settings = egui_extras::syntax_highlighting::SyntectSettings::default();
        for lang in CodeLanguage::ALL {
            assert!(
                settings.ps.find_syntax_by_name(lang.syntax_key()).is_some()
                    || settings
                        .ps
                        .find_syntax_by_extension(lang.syntax_key())
                        .is_some(),
                "{} ({}) has no syntax definition",
                lang.label(),
                lang.syntax_key()
            );
        }
    }
}
