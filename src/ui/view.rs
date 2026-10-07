//! Read-only renderers shared by the desktop panes and the browser client, so a
//! task looks the same in both. Everything here takes plain data (`&Task`,
//! `&Annotation`) — never `FastTask` or the database.

use bson::DateTime;

use crate::database::models::{
    ActivityAction, ActivityEntry, Annotation, Priority, Task, TaskStatus,
};
use crate::ui::theme::{colors, icons};
use crate::ui::widgets::{code, common};

/// Nerd Font glyph for a status.
pub fn status_icon(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::NotStarted => icons::STATUS_NOT_STARTED,
        TaskStatus::InProgress => icons::STATUS_IN_PROGRESS,
        TaskStatus::OnHold => icons::STATUS_ON_HOLD,
        TaskStatus::Completed => icons::STATUS_COMPLETED,
    }
}

/// Human name for a status ("In progress").
pub fn status_name(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::NotStarted => "Not started",
        TaskStatus::InProgress => "In progress",
        TaskStatus::OnHold => "On hold",
        TaskStatus::Completed => "Completed",
    }
}

/// Glyph shown next to a task's title for its priority; empty for Normal.
pub fn priority_icon(priority: &Priority) -> &'static str {
    match priority {
        Priority::Urgent => icons::PRIORITY_URGENT,
        Priority::Normal => "",
        Priority::Low => icons::PRIORITY_LOW,
    }
}

/// True while a task's `wait_until` is still in the future (it stays hidden).
pub fn is_waiting(task: &Task, now_ms: i64) -> bool {
    task.wait_until
        .is_some_and(|wait| wait.timestamp_millis() > now_ms)
}

pub use crate::database::models::bson_dt_to_jiff_date;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month_name(month: i8) -> &'static str {
    MONTHS
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("?")
}

/// Formats a due date as "Mon D" (current year) or "Mon D, YYYY" (other years).
pub fn format_due_short(dt: &DateTime) -> String {
    let Some(date) = bson_dt_to_jiff_date(dt) else {
        return String::new();
    };
    let today = jiff::Zoned::now().date();
    let month = month_name(date.month());
    if date.year() == today.year() {
        format!("{} {}", month, date.day())
    } else {
        format!("{} {}, {}", month, date.day(), date.year())
    }
}

/// Red if overdue, yellow if due today, muted otherwise.
pub fn due_date_color(dt: &DateTime) -> egui::Color32 {
    let Some(date) = bson_dt_to_jiff_date(dt) else {
        return colors::SUBTEXT0;
    };
    let today = jiff::Zoned::now().date();
    if date < today {
        colors::RED
    } else if date == today {
        colors::YELLOW
    } else {
        colors::SUBTEXT0
    }
}

/// Formats an annotation timestamp as "Jun 2, 14:03" in local time.
pub fn format_annotation_ts(dt: &DateTime) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(dt.timestamp_millis()) else {
        return String::new();
    };
    let z = ts.to_zoned(jiff::tz::TimeZone::system());
    format!(
        "{} {}, {:02}:{:02}",
        month_name(z.month()),
        z.day(),
        z.hour(),
        z.minute()
    )
}

/// Read-only summary card for a task — title, details, and metadata grid.
/// Used in the Info pane; the bottom detail pane uses the compact `task_summary`.
pub fn task_card(ui: &mut egui::Ui, task: &Task) {
    ui.label(egui::RichText::new(&task.title).size(16.0).strong());
    ui.separator();
    if !task.details.is_empty() {
        egui::ScrollArea::vertical()
            .id_salt("task_card_details")
            .show(ui, |ui| {
                code::details_view(ui, &task.details, task.code, task.language);
            });
        ui.add_space(4.0);
    }
    egui::Grid::new("task_card_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            common::field_label(ui, "Status");
            common::status_badge(ui, &task.status);
            ui.end_row();

            common::field_label(ui, "Priority");
            ui.label(
                egui::RichText::new(task.priority.to_string())
                    .color(crate::ui::theme::priority_color(&task.priority)),
            );
            ui.end_row();

            if let Some(due) = task.due {
                common::field_label(ui, "Due");
                ui.label(egui::RichText::new(format_due_short(&due)).color(due_date_color(&due)));
                ui.end_row();
            }

            if let Some(tags) = &task.tags
                && !tags.is_empty()
            {
                common::field_label(ui, "Tags");
                ui.label(tags.join(", "));
                ui.end_row();
            }

            if let Some(wait) = task.wait_until {
                common::field_label(ui, "Hidden until");
                ui.label(egui::RichText::new(format_due_short(&wait)).color(colors::SUBTEXT0));
                ui.end_row();
            }

            if let Some(recurrence) = &task.recurrence {
                common::field_label(ui, "Recurrence");
                ui.label(egui::RichText::new(recurrence.to_string()).color(colors::TEAL));
                ui.end_row();
            }
        });
}

/// Compact at-a-glance summary for the bottom detail pane: title plus one
/// wrapped row of status / priority / due / tags. The details body and notes
/// are left to the Info pane's full `task_card`.
pub fn task_summary(ui: &mut egui::Ui, task: &Task) {
    ui.label(egui::RichText::new(&task.title).size(14.0).strong());
    ui.horizontal_wrapped(|ui| {
        common::status_badge(ui, &task.status);
        ui.label(
            egui::RichText::new(task.priority.to_string())
                .color(crate::ui::theme::priority_color(&task.priority)),
        )
        .on_hover_text("Priority");
        if let Some(due) = task.due {
            ui.label(egui::RichText::new(format_due_short(&due)).color(due_date_color(&due)))
                .on_hover_text("Due");
        }
        if let Some(tags) = &task.tags {
            for tag in tags {
                ui.label(egui::RichText::new(format!("#{tag}")).color(colors::SUBTEXT0));
            }
        }
        if !task.details.is_empty() {
            ui.label(egui::RichText::new("…").color(colors::OVERLAY1))
                .on_hover_text("Has details — open the task (i / e) to read them");
        }
    });
}

/// A row's edit / complete icon, clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowIcon {
    Edit,
    Complete,
}

/// One clickable task-list row: status glyph, title, priority glyph, and the due
/// date on the right — the same pieces and colors as the desktop task table.
/// `selected` paints the cursor highlight. With `actions`, a hovered or selected
/// row shows edit / complete icons in the due date's place, as the desktop does
/// on hover (selected too, since touch screens have no hover).
pub fn task_row(
    ui: &mut egui::Ui,
    task: &Task,
    selected: bool,
    actions: bool,
) -> (egui::Response, Option<RowIcon>) {
    let height = 28.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    // Pointer test rather than `hovered()`: stays true over the icons drawn on
    // top, so they don't flicker.
    let hovered = ui.rect_contains_pointer(rect);
    if selected {
        ui.painter().rect_filled(rect, 3.0, colors::BLUE);
    } else if hovered {
        ui.painter().rect_filled(rect, 3.0, colors::SURFACE1);
    }
    // Highlighted rows switch every piece to MANTLE, as on the desktop.
    let pick = |c| if selected { colors::MANTLE } else { c };
    let title_color = if task.status == TaskStatus::Completed {
        colors::SUBTEXT0
    } else {
        colors::TEXT
    };

    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(6.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let mut clicked = None;
    // One scope either way, so icons vs. due date don't shift the title's ids.
    row.scope(|ui| {
        if actions && (hovered || selected) {
            let icon = |ui: &mut egui::Ui, glyph: &str, color| {
                let text = egui::RichText::new(glyph).color(pick(color)).size(14.0);
                ui.add(egui::Button::new(text).frame(false))
            };
            // right_to_left: first added sits rightmost, as on the desktop.
            if task.status != TaskStatus::Completed
                && icon(ui, icons::STATUS_COMPLETED, colors::GREEN)
                    .on_hover_text("Complete")
                    .clicked()
            {
                clicked = Some(RowIcon::Complete);
            }
            if icon(ui, icons::MODE_INSERT, colors::BLUE)
                .on_hover_text("Edit")
                .clicked()
            {
                clicked = Some(RowIcon::Edit);
            }
        } else if let Some(due) = &task.due {
            ui.label(
                egui::RichText::new(format_due_short(due))
                    .color(pick(due_date_color(due)))
                    .size(11.0),
            );
        }
    });
    row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(
            egui::RichText::new(status_icon(&task.status))
                .color(pick(crate::ui::theme::status_color(&task.status))),
        );
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("  {}", task.title)).color(pick(title_color)),
            )
            .truncate()
            .selectable(false),
        );
        let priority = priority_icon(&task.priority);
        if !priority.is_empty() {
            ui.label(
                egui::RichText::new(priority)
                    .color(pick(crate::ui::theme::priority_color(&task.priority))),
            );
        }
    });
    (response, clicked)
}

/// Read-only notes list, oldest first. `author` labels a note's writer (see
/// [`note_card`]).
pub fn notes_list(
    ui: &mut egui::Ui,
    notes: &[Annotation],
    author: impl Fn(&Option<String>) -> Option<(String, egui::Color32)>,
) {
    if notes.is_empty() {
        ui.label(
            egui::RichText::new("No notes yet.")
                .color(colors::OVERLAY1)
                .size(11.0)
                .italics(),
        );
    }
    for note in notes {
        note_card(ui, note, author(&note.author), false, false);
    }
}

/// What a click on a [`note_card`] did.
pub struct NoteCard {
    pub response: egui::Response,
    pub delete: bool,
}

/// A note as a small card: author (when there is one) and time on top, the
/// text below, and — if `deletable` — a small delete icon top right.
/// `highlighted` outlines it (the desktop's keyboard cursor).
pub fn note_card(
    ui: &mut egui::Ui,
    note: &Annotation,
    author: Option<(String, egui::Color32)>,
    highlighted: bool,
    deletable: bool,
) -> NoteCard {
    let mut delete = false;
    let stroke = if highlighted {
        egui::Stroke::new(1.0_f32, colors::BLUE)
    } else {
        egui::Stroke::new(1.0_f32, colors::SURFACE1)
    };
    let response = egui::Frame::new()
        .fill(colors::MANTLE)
        .stroke(stroke)
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some((name, color)) = author {
                    ui.label(egui::RichText::new(name).color(color).strong().size(11.5));
                }
                ui.label(
                    egui::RichText::new(format_annotation_ts(&note.created_at))
                        .color(colors::OVERLAY1)
                        .size(10.5),
                );
                if deletable {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        delete = small_icon_button(ui, icons::DELETE, colors::RED)
                            .on_hover_text("Delete note")
                            .clicked();
                    });
                }
            });
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(egui::RichText::new(&note.content).size(12.5))
                    .wrap()
                    .selectable(true),
            );
        })
        .response;
    ui.add_space(6.0);
    NoteCard { response, delete }
}

/// An icon for a toggle (`selectable_label`): dark when selected, since the
/// selected fill is a light blue that washes out the normal text color.
pub fn selectable_icon(icon: &str, selected: bool) -> egui::RichText {
    egui::RichText::new(icon).color(if selected {
        colors::MANTLE
    } else {
        colors::TEXT
    })
}

/// A quiet, frameless icon button, sized to sit inline with small text; it
/// takes `hover` color under the pointer.
pub fn small_icon_button(ui: &mut egui::Ui, icon: &str, hover: egui::Color32) -> egui::Response {
    let id = ui.next_auto_id();
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let color = if hovered { hover } else { colors::OVERLAY1 };
    ui.add(
        egui::Button::new(egui::RichText::new(icon).size(12.0).color(color))
            .frame(false)
            .min_size(egui::vec2(16.0, 16.0)),
    )
}

/// A stable color per person, so the same name always reads the same.
pub fn person_color(name: &str) -> egui::Color32 {
    const PALETTE: [egui::Color32; 5] = [
        colors::PEACH,
        colors::GREEN,
        colors::MAUVE,
        colors::TEAL,
        colors::YELLOW,
    ];
    let hash = name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    PALETTE[hash as usize % PALETTE.len()]
}

/// "Today 14:02", "Yesterday 14:02", "Oct 3 14:02", or with the year when
/// it isn't this one (local time).
pub fn activity_time(dt: &bson::DateTime) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(dt.timestamp_millis()) else {
        return String::new();
    };
    let tz = jiff::tz::TimeZone::system();
    let z = ts.to_zoned(tz.clone());
    let today = jiff::Timestamp::now().to_zoned(tz).date();
    let time = format!("{:02}:{:02}", z.hour(), z.minute());
    let date = z.date();
    if date == today {
        format!("Today {time}")
    } else if today.yesterday().ok() == Some(date) {
        format!("Yesterday {time}")
    } else if date.year() == today.year() {
        format!("{} {} {time}", month_name(date.month()), date.day())
    } else {
        format!(
            "{} {} {}, {time}",
            month_name(date.month()),
            date.day(),
            date.year()
        )
    }
}

/// Icon, label and color of an activity chip.
pub fn action_style(action: ActivityAction) -> (&'static str, &'static str, egui::Color32) {
    use ActivityAction::*;
    match action {
        Added => (icons::NEW, "Added", colors::GREEN),
        Completed => (icons::STATUS_COMPLETED, "Completed", colors::GREEN),
        Started => (icons::STATUS_IN_PROGRESS, "Started", colors::BLUE),
        Reopened => (icons::STATUS_NOT_STARTED, "Reopened", colors::SUBTEXT0),
        OnHold => (icons::STATUS_ON_HOLD, "On hold", colors::YELLOW),
        Renamed => (icons::MODE_INSERT, "Renamed", colors::BLUE),
        Edited => (icons::MODE_INSERT, "Edited", colors::BLUE),
        Priority => (icons::PRIORITY_URGENT, "Priority", colors::PEACH),
        Moved => (icons::MOVE, "Moved", colors::OVERLAY1),
        Deleted => (icons::DELETE, "Deleted", colors::RED),
        Noted => (icons::NOTE, "Note", colors::MAUVE),
        NoteDeleted => (icons::DELETE, "Note deleted", colors::RED),
        Undid => (icons::UNDO, "Undo", colors::LAVENDER),
        Redid => (icons::REDO, "Redo", colors::LAVENDER),
    }
}

/// The share activity log as cards, newest first: who and when on top, a
/// colored action chip and the task below, then any detail. `label` names an
/// entry's author from the viewer's point of view ("You" for their own edits).
pub fn activity_list(
    ui: &mut egui::Ui,
    entries: &[ActivityEntry],
    label: impl Fn(&Option<String>) -> (String, egui::Color32),
) {
    for entry in entries {
        let (who, who_color) = label(&entry.who);
        egui::Frame::new()
            .fill(colors::MANTLE)
            .stroke(egui::Stroke::new(1.0_f32, colors::SURFACE1))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(who)
                            .color(who_color)
                            .strong()
                            .size(12.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(activity_time(&entry.at))
                                .color(colors::OVERLAY1)
                                .size(10.5),
                        );
                    });
                });
                ui.add_space(3.0);
                match (entry.action, &entry.subject) {
                    (Some(action), Some(subject)) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            action_chip(ui, action);
                            ui.label(egui::RichText::new(subject).size(12.5));
                        });
                        if let Some(detail) = &entry.detail {
                            ui.label(
                                egui::RichText::new(detail)
                                    .color(colors::SUBTEXT0)
                                    .size(11.0),
                            );
                        }
                    }
                    // Logged before entries had an action.
                    _ => {
                        ui.label(egui::RichText::new(&entry.what).size(12.5));
                    }
                }
            });
        ui.add_space(6.0);
    }
}

fn action_chip(ui: &mut egui::Ui, action: ActivityAction) {
    let (icon, label, color) = action_style(action);
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(format!("{icon} {label}"))
                    .color(color)
                    .size(11.0)
                    .strong(),
            );
        });
}
