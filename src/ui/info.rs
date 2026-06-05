use egui::InnerResponse;
use polodb_core::bson::oid::ObjectId;

use crate::database::models::{Annotation, Priority, Recurrence, TaskStatus};
use crate::ui::app::{DB, EditFocus, FastTask, Mode, UpdateMessage, WindowState};
use crate::ui::tasks::{
    TaskWriter, due_date_color, format_due_short, task_submit_create, task_submit_edit,
};
use crate::ui::theme::colors;
use crate::ui::widgets::common;

/// Formats an annotation timestamp as "Jun 2, 14:03" in local time.
fn format_annotation_ts(dt: &polodb_core::bson::DateTime) -> String {
    let Ok(ts) = jiff::Timestamp::from_millisecond(dt.timestamp_millis()) else {
        return String::new();
    };
    let z = ts.to_zoned(jiff::tz::TimeZone::system());
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .get((z.month() as usize).saturating_sub(1))
    .copied()
    .unwrap_or("?");
    format!("{} {}, {:02}:{:02}", month, z.day(), z.hour(), z.minute())
}

/// Renders a date-entry row: free-text field + calendar picker + optional clear button.
/// Updates `*date` and clears `text_buf` when the picker fires.
/// Shows a confirmation label (or "unrecognized date") below the row.
fn date_field(
    ui: &mut egui::Ui,
    text_buf: &mut String,
    date: &mut Option<polodb_core::bson::DateTime>,
    id_salt: &str,
    confirmed_text: impl Fn(&polodb_core::bson::DateTime) -> (String, egui::Color32),
) {
    use crate::ui::tasks::{bson_dt_to_jiff_date, from_jiff_to_datetime};
    use crate::ui::theme::colors;
    use crate::ui::widgets::common;

    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(text_buf)
                .desired_width(110.0)
                .hint_text("today, fri, 2026-06-15"),
        );
        if resp.changed() {
            if text_buf.is_empty() {
                *date = None;
            } else if let Some(d) = parse_due_text(text_buf) {
                *date = from_jiff_to_datetime(d);
            }
        }

        let now = jiff::Zoned::now();
        let mut cal_date = date
            .as_ref()
            .and_then(bson_dt_to_jiff_date)
            .unwrap_or_else(|| now.date());
        let this_year = now.year();
        let picker = ui.add(
            egui_extras::DatePickerButton::new(&mut cal_date)
                .id_salt(id_salt)
                .show_icon(true)
                .start_end_years(this_year..=this_year + 10),
        );
        if picker.changed() {
            *date = from_jiff_to_datetime(cal_date);
            text_buf.clear();
        }

        if date.is_some()
            && common::secondary_button(ui, "✕")
                .on_hover_text("Clear date")
                .clicked()
        {
            *date = None;
            text_buf.clear();
        }
    });

    if let Some(dt) = date.as_ref() {
        if text_buf.is_empty() {
            let (text, color) = confirmed_text(dt);
            ui.label(egui::RichText::new(text).color(color).size(11.0));
        }
    } else if !text_buf.is_empty() {
        ui.label(
            egui::RichText::new("unrecognized date")
                .color(colors::OVERLAY0)
                .size(11.0)
                .italics(),
        );
    }
}

fn parse_due_text(text: &str) -> Option<jiff::civil::Date> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }

    let today = jiff::Zoned::now().date();
    let one_day = jiff::Span::new().days(1i64);

    match text.as_str() {
        "today" | "tod" => return Some(today),
        "tomorrow" | "tmr" | "tom" => return today.checked_add(one_day).ok(),
        _ => {}
    }

    let weekday = match text.as_str() {
        "mon" | "monday" => Some(jiff::civil::Weekday::Monday),
        "tue" | "tuesday" => Some(jiff::civil::Weekday::Tuesday),
        "wed" | "wednesday" => Some(jiff::civil::Weekday::Wednesday),
        "thu" | "thursday" => Some(jiff::civil::Weekday::Thursday),
        "fri" | "friday" => Some(jiff::civil::Weekday::Friday),
        "sat" | "saturday" => Some(jiff::civil::Weekday::Saturday),
        "sun" | "sunday" => Some(jiff::civil::Weekday::Sunday),
        _ => None,
    };
    if let Some(wd) = weekday {
        let mut d = today.checked_add(one_day).ok()?;
        for _ in 0..7 {
            if d.weekday() == wd {
                return Some(d);
            }
            d = d.checked_add(one_day).ok()?;
        }
        return None;
    }

    // YYYY-MM-DD
    let parts: Vec<&str> = text.splitn(3, '-').collect();
    if parts.len() == 3 {
        let y = parts[0].parse::<i16>().ok()?;
        let m = parts[1].parse::<i8>().ok()?;
        let d = parts[2].parse::<i8>().ok()?;
        return jiff::civil::Date::new(y, m, d).ok();
    }

    None
}

pub fn info_state(ui: &mut egui::Ui, app: &mut FastTask) -> InnerResponse<()> {
    egui::CentralPanel::default().show_inside(ui, |ui| match &app.app_state.mode {
        Mode::Insert(_) => info_editor(ui, app),
        Mode::Normal | Mode::Visual => {
            if let Some(task) = app.task_manager.get_current_task() {
                // Detect task change: set id synchronously then spawn a fetch.
                if app.annotation_task_id != Some(task.id) {
                    app.annotation_task_id = Some(task.id);
                    app.annotations.clear();
                    app.annotation_buf.clear();
                    let task_id = task.id;
                    let tx = app.backend_manager.tx.clone();
                    std::thread::spawn(move || match DB.get_annotations(task_id) {
                        Ok(anns) => {
                            tx.send(UpdateMessage::Annotations(task_id, anns)).ok();
                        }
                        Err(e) => {
                            tx.send(UpdateMessage::Error(e)).ok();
                        }
                    });
                }

                egui::Frame::new()
                    .inner_margin(egui::Margin::same(12_i8))
                    .show(ui, |ui| {
                        common::heading(ui, "Task Info");
                        ui.add_space(4.0);
                        crate::ui::tasks::task_card(ui, &task);

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);

                        common::field_label(ui, "Notes");

                        // Scrollable annotation list
                        let mut to_delete: Option<ObjectId> = None;
                        egui::ScrollArea::vertical()
                            .id_salt("annotations_scroll")
                            .max_height(180.0)
                            .show(ui, |ui| {
                                if app.annotations.is_empty() {
                                    ui.label(
                                        egui::RichText::new("No notes yet.")
                                            .color(colors::OVERLAY1)
                                            .size(11.0)
                                            .italics(),
                                    );
                                }
                                for ann in &app.annotations {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(format_annotation_ts(
                                                &ann.created_at,
                                            ))
                                            .color(colors::SUBTEXT0)
                                            .size(11.0),
                                        );
                                        if common::secondary_button(
                                            ui,
                                            crate::ui::theme::icons::DISCARD,
                                        )
                                        .on_hover_text("Delete note")
                                        .clicked()
                                        {
                                            to_delete = Some(ann.id);
                                        }
                                    });
                                    ui.label(egui::RichText::new(&ann.content).size(12.0));
                                    ui.add_space(4.0);
                                }
                            });

                        if let Some(ann_id) = to_delete {
                            let task_id = task.id;
                            let tx = app.backend_manager.tx.clone();
                            std::thread::spawn(move || {
                                if let Err(e) = DB.delete_annotation(ann_id) {
                                    let _ = tx.send(UpdateMessage::Error(e));
                                    return;
                                }
                                match DB.get_annotations(task_id) {
                                    Ok(anns) => {
                                        tx.send(UpdateMessage::Annotations(task_id, anns)).ok();
                                    }
                                    Err(e) => {
                                        tx.send(UpdateMessage::Error(e)).ok();
                                    }
                                }
                            });
                        }

                        // New-note input
                        ui.add_space(4.0);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut app.annotation_buf)
                                .desired_width(f32::INFINITY)
                                .hint_text("Add a note… (Enter to save)"),
                        );
                        // Esc surrenders focus without saving
                        if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            ui.ctx().memory_mut(|m| m.surrender_focus(resp.id));
                            app.annotation_buf.clear();
                        }
                        // Enter saves
                        if resp.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !app.annotation_buf.is_empty()
                        {
                            let annotation = Annotation {
                                id: ObjectId::new(),
                                task_id: task.id,
                                content: std::mem::take(&mut app.annotation_buf),
                                created_at: polodb_core::bson::DateTime::now(),
                            };
                            let task_id = task.id;
                            let tx = app.backend_manager.tx.clone();
                            std::thread::spawn(move || {
                                if let Err(e) = DB.add_annotation(annotation) {
                                    let _ = tx.send(UpdateMessage::Error(e));
                                    return;
                                }
                                match DB.get_annotations(task_id) {
                                    Ok(anns) => {
                                        tx.send(UpdateMessage::Annotations(task_id, anns)).ok();
                                    }
                                    Err(e) => {
                                        tx.send(UpdateMessage::Error(e)).ok();
                                    }
                                }
                            });
                        }
                    });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(
                            "No task selected.\no / O  — new task below / above\nj / k  — navigate",
                        )
                        .color(colors::SUBTEXT0)
                        .size(11.0),
                    );
                });
            }
        }
    })
}

fn info_editor(ui: &mut egui::Ui, app: &mut FastTask) {
    if let Mode::Insert(Some(_id)) = &app.app_state.mode
        && app.task_manager.writer.initial_frame
        && let Some(task) = app.task_manager.get_current_task()
    {
        app.task_manager.writer = TaskWriter::from(task);
    }

    // Fetch annotations for the task being edited if not yet loaded
    if let Mode::Insert(Some(task_id)) = &app.app_state.mode {
        let task_id = *task_id;
        if app.annotation_task_id != Some(task_id) {
            app.annotation_task_id = Some(task_id);
            app.annotations.clear();
            let tx = app.backend_manager.tx.clone();
            std::thread::spawn(move || match DB.get_annotations(task_id) {
                Ok(anns) => {
                    tx.send(UpdateMessage::Annotations(task_id, anns)).ok();
                }
                Err(e) => {
                    tx.send(UpdateMessage::Error(e)).ok();
                }
            });
        }
    }

    let is_edit = matches!(&app.app_state.mode, Mode::Insert(Some(_)));

    egui::Frame::new()
        .inner_margin(egui::Margin::same(12_i8))
        .show(ui, |ui| {
            {
                use crate::ui::theme::icons;
                let (icon, label) = if is_edit {
                    (icons::MODE_INSERT, "Edit Task")
                } else {
                    (icons::NEW, "New Task")
                };
                common::heading(ui, format!("{icon}  {label}"));
            }
            ui.add_space(8.0);

            // Buttons pinned at bottom; scroll area fills the middle.
            let scroll_h = (ui.available_height() - 64.0).max(100.0);

            egui::ScrollArea::vertical()
                .max_height(scroll_h)
                .show(ui, |ui| {
                    // Title
                    common::field_label(ui, "Title");
                    let header = ui.add(
                        egui::TextEdit::singleline(&mut app.task_manager.writer.title_buffer)
                            .desired_width(f32::INFINITY)
                            .hint_text("What needs to be done?"),
                    );

                    ui.add_space(4.0);

                    // Details — code-mode toggle inline with the label
                    ui.horizontal(|ui| {
                        common::field_label(ui, "Details");
                        ui.add_space(8.0);
                        let code = &mut app.task_manager.writer.code;
                        ui.checkbox(
                            code,
                            egui::RichText::new("Code")
                                .size(11.0)
                                .color(colors::SUBTEXT0),
                        );
                    });
                    if app.task_manager.writer.code {
                        ui.code_editor(&mut app.task_manager.writer.details_buffer);
                    } else {
                        ui.add(
                            egui::TextEdit::multiline(&mut app.task_manager.writer.details_buffer)
                                .desired_width(f32::INFINITY)
                                .desired_rows(4)
                                .hint_text("Additional context…"),
                        );
                    }

                    // Auto-focus title on first frame (must be inside scroll area to capture header)
                    if let EditFocus::None = app.task_manager.writer.has_focus {
                        header.request_focus();
                        app.task_manager.writer.has_focus = EditFocus::Header;
                    }
                    app.task_manager.writer.initial_frame = false;

                    ui.add_space(4.0);

                    // Tags
                    common::field_label(ui, "Tags");
                    {
                        let tags: Vec<String> = app
                            .task_manager
                            .writer
                            .tags_buffer
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                        if !tags.is_empty() {
                            let mut to_remove: Option<usize> = None;
                            ui.horizontal_wrapped(|ui| {
                                for (i, tag) in tags.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        ui.label(
                                            egui::RichText::new(tag).color(colors::TEAL).size(11.0),
                                        );
                                        if common::secondary_button(ui, "✕")
                                            .on_hover_text("Remove tag")
                                            .clicked()
                                        {
                                            to_remove = Some(i);
                                        }
                                    });
                                    ui.add_space(4.0);
                                }
                            });
                            if let Some(idx) = to_remove {
                                let remaining: Vec<String> = tags
                                    .into_iter()
                                    .enumerate()
                                    .filter(|(i, _)| *i != idx)
                                    .map(|(_, t)| t)
                                    .collect();
                                app.task_manager.writer.tags_buffer = if remaining.is_empty() {
                                    String::new()
                                } else {
                                    format!("{}, ", remaining.join(", "))
                                };
                            }
                        }
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut app.task_manager.writer.tags_buffer)
                            .desired_width(f32::INFINITY)
                            .hint_text("work, urgent, …  (comma-separated)"),
                    );

                    // Autocomplete suggestions
                    {
                        let partial = app
                            .task_manager
                            .writer
                            .tags_buffer
                            .rsplit(',')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_lowercase();

                        if !partial.is_empty() && !app.known_tags.is_empty() {
                            let suggestions: Vec<String> = app
                                .known_tags
                                .iter()
                                .filter(|t| t.to_lowercase().starts_with(&partial))
                                .take(5)
                                .cloned()
                                .collect();

                            if !suggestions.is_empty() {
                                let mut chosen: Option<String> = None;
                                egui::Frame::new()
                                    .fill(colors::SURFACE0)
                                    .stroke(egui::Stroke::new(1.0, colors::SURFACE1))
                                    .inner_margin(egui::Margin::same(4_i8))
                                    .show(ui, |ui| {
                                        ui.set_min_width(ui.available_width());
                                        for suggestion in &suggestions {
                                            if ui
                                                .selectable_label(
                                                    false,
                                                    egui::RichText::new(suggestion)
                                                        .color(colors::TEAL)
                                                        .size(11.0),
                                                )
                                                .clicked()
                                            {
                                                chosen = Some(suggestion.clone());
                                            }
                                        }
                                    });

                                if let Some(tag) = chosen {
                                    let buf = &mut app.task_manager.writer.tags_buffer;
                                    if let Some(last_comma) = buf.rfind(',') {
                                        buf.truncate(last_comma + 1);
                                        buf.push(' ');
                                        buf.push_str(&tag);
                                        buf.push_str(", ");
                                    } else {
                                        *buf = format!("{}, ", tag);
                                    }
                                }
                            }
                        }
                    }

                    ui.add_space(4.0);

                    // Priority
                    common::field_label(ui, "Priority");
                    ui.horizontal(|ui| {
                        use crate::ui::theme::icons;
                        let priority_label = match app.task_manager.writer.priority {
                            Priority::Low => format!("{}  Low", icons::PRIORITY_LOW),
                            Priority::Normal => "Normal".to_string(),
                            Priority::Urgent => format!("{}  Urgent", icons::PRIORITY_URGENT),
                        };
                        egui::ComboBox::from_id_salt("priority_combo")
                            .selected_text(priority_label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut app.task_manager.writer.priority,
                                    Priority::Low,
                                    format!("{}  Low", icons::PRIORITY_LOW),
                                );
                                ui.selectable_value(
                                    &mut app.task_manager.writer.priority,
                                    Priority::Normal,
                                    "Normal",
                                );
                                ui.selectable_value(
                                    &mut app.task_manager.writer.priority,
                                    Priority::Urgent,
                                    format!("{}  Urgent", icons::PRIORITY_URGENT),
                                );
                            });
                    });

                    ui.add_space(4.0);

                    // Status
                    common::field_label(ui, "Status");
                    ui.horizontal_wrapped(|ui| {
                        use crate::ui::theme::{icons, status_color};
                        const STATUSES: [(TaskStatus, &str, &str); 4] = [
                            (TaskStatus::NotStarted, icons::STATUS_NOT_STARTED, "None"),
                            (TaskStatus::InProgress, icons::STATUS_IN_PROGRESS, "Active"),
                            (TaskStatus::OnHold, icons::STATUS_ON_HOLD, "Hold"),
                            (TaskStatus::Completed, icons::STATUS_COMPLETED, "Done"),
                        ];
                        for (status, icon, label) in STATUSES {
                            let selected = app.task_manager.writer.status == status;
                            let color = status_color(&status);
                            let text = egui::RichText::new(format!("{icon} {label}"))
                                .color(if selected { colors::MANTLE } else { color })
                                .size(11.0);
                            let btn = egui::Button::new(text)
                                .fill(if selected { color } else { colors::SURFACE0 })
                                .stroke(egui::Stroke::new(1.0, color));
                            if ui.add(btn).clicked() {
                                app.task_manager.writer.status = status;
                            }
                        }
                    });

                    ui.add_space(4.0);

                    // Due date
                    common::field_label(ui, "Due date");
                    date_field(
                        ui,
                        &mut app.task_manager.writer.due_text,
                        &mut app.task_manager.writer.duedate,
                        "due_date_picker",
                        |dt| (format_due_short(dt), due_date_color(dt)),
                    );

                    ui.add_space(4.0);

                    // Wait until (hidden-until date)
                    common::field_label(ui, "Wait until  (hide until this date)");
                    date_field(
                        ui,
                        &mut app.task_manager.writer.wait_text,
                        &mut app.task_manager.writer.wait_until,
                        "wait_until_picker",
                        |dt| {
                            (
                                format!("hidden until {}", format_due_short(dt)),
                                colors::OVERLAY1,
                            )
                        },
                    );

                    ui.add_space(4.0);

                    // Recurrence
                    common::field_label(ui, "Recurrence");
                    let recur_label = match &app.task_manager.writer.recurrence {
                        None => "None".to_string(),
                        Some(r) => r.to_string(),
                    };
                    egui::ComboBox::from_id_salt("recurrence_combo")
                        .selected_text(recur_label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut app.task_manager.writer.recurrence,
                                None,
                                "None",
                            );
                            ui.selectable_value(
                                &mut app.task_manager.writer.recurrence,
                                Some(Recurrence::Daily),
                                "Daily",
                            );
                            ui.selectable_value(
                                &mut app.task_manager.writer.recurrence,
                                Some(Recurrence::Weekly),
                                "Weekly",
                            );
                            ui.selectable_value(
                                &mut app.task_manager.writer.recurrence,
                                Some(Recurrence::Monthly),
                                "Monthly",
                            );
                            ui.selectable_value(
                                &mut app.task_manager.writer.recurrence,
                                Some(Recurrence::Yearly),
                                "Yearly",
                            );
                        });
                }); // end ScrollArea

            // Actions — always visible below the scroll area
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                use crate::ui::theme::icons;
                if common::primary_button(ui, format!("{}  Save", icons::SAVE)).clicked() {
                    submit(app);
                }
                if common::secondary_button(ui, format!("{}  Discard", icons::DISCARD)).clicked() {
                    discard(app);
                }
                if common::secondary_button(ui, format!("{}  Done", icons::STATUS_COMPLETED))
                    .clicked()
                {
                    app.task_manager.writer.status = TaskStatus::Completed;
                    submit(app);
                }
            });
        }); // end Frame

    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if enter {
        submit(app);
    }
    if esc {
        discard(app);
    }
}

fn submit(app: &mut FastTask) {
    let writer = app.task_manager.writer.clone();
    let tx = app.backend_manager.tx.clone();
    let project = app.project_manager.current().clone();

    let backend = app.backend_manager.backend.clone();
    match app.app_state.mode {
        Mode::Insert(None) => task_submit_create(backend, writer, project, tx),
        Mode::Insert(Some(id)) => task_submit_edit(backend, writer, id, tx),
        _ => {}
    }

    app.task_manager.writer.flush();
    app.app_state.mode = Mode::Normal;
    app.app_state.window_state = WindowState::Tasks;
}

fn discard(app: &mut FastTask) {
    app.task_manager.writer.flush();
    app.app_state.mode = Mode::Normal;
    app.app_state.window_state = WindowState::Tasks;
}
