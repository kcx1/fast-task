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
            && common::secondary_button(ui, crate::ui::theme::icons::DISCARD)
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
                .color(colors::YELLOW)
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
                    app.annotation_cursor = None;
                    let task_id = task.id;
                    let tx = app.backend_manager.tx.clone();
                    crate::ui::bg::spawn(move || match DB.get_annotations(task_id) {
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
                        ui.horizontal(|ui| {
                            common::heading(ui, "Task Info");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if common::secondary_button(
                                        ui,
                                        format!("{}  Edit", crate::ui::theme::icons::MODE_INSERT),
                                    )
                                    .on_hover_text("Edit this task (i / e)")
                                    .clicked()
                                    {
                                        app.app_state.mode = Mode::Insert(Some(task.id));
                                        app.app_state.window_state = WindowState::Info;
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        crate::ui::tasks::task_card(ui, &task);

                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);

                        common::field_label(ui, "Notes");

                        // Info-pane keys (only when no text field has focus):
                        // e / i edit, a add note, j / k / ↓ / ↑ highlight a note, x delete it.
                        let keys_free = !ui.ctx().egui_wants_keyboard_input();
                        let (key_edit, key_add, key_down, key_up, key_del) = if keys_free {
                            ui.input(|i| {
                                (
                                    i.key_pressed(egui::Key::E) || i.key_pressed(egui::Key::I),
                                    i.key_pressed(egui::Key::A) && !i.modifiers.shift,
                                    i.key_pressed(egui::Key::J)
                                        || i.key_pressed(egui::Key::ArrowDown),
                                    i.key_pressed(egui::Key::K)
                                        || i.key_pressed(egui::Key::ArrowUp),
                                    i.key_pressed(egui::Key::X),
                                )
                            })
                        } else {
                            (false, false, false, false, false)
                        };
                        if key_edit {
                            app.app_state.mode = Mode::Insert(Some(task.id));
                            app.app_state.window_state = WindowState::Info;
                        }
                        let n_notes = app.annotations.len();
                        if key_down && n_notes > 0 {
                            app.annotation_cursor = Some(
                                app.annotation_cursor
                                    .map_or(0, |c| (c + 1).min(n_notes - 1)),
                            );
                        }
                        if key_up && n_notes > 0 {
                            app.annotation_cursor =
                                Some(app.annotation_cursor.map_or(0, |c| c.saturating_sub(1)));
                        }
                        if app.annotation_cursor.is_some_and(|c| c >= n_notes) {
                            app.annotation_cursor = n_notes.checked_sub(1);
                        }

                        // Scrollable annotation list
                        let mut to_delete: Option<ObjectId> = if key_del {
                            app.annotation_cursor
                                .and_then(|c| app.annotations.get(c))
                                .map(|a| a.id)
                        } else {
                            None
                        };
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
                                for (idx, ann) in app.annotations.iter().enumerate() {
                                    let highlighted = app.annotation_cursor == Some(idx);
                                    ui.horizontal(|ui| {
                                        let ts = format_annotation_ts(&ann.created_at);
                                        let (ts, ts_color) = if highlighted {
                                            (format!("▸ {ts}"), colors::BLUE)
                                        } else {
                                            (ts, colors::SUBTEXT0)
                                        };
                                        let ts_resp = ui.label(
                                            egui::RichText::new(ts).color(ts_color).size(11.0),
                                        );
                                        if highlighted && (key_down || key_up) {
                                            ts_resp.scroll_to_me(None);
                                        }
                                        if common::secondary_button(
                                            ui,
                                            crate::ui::theme::icons::DISCARD,
                                        )
                                        .on_hover_text("Delete note (x)")
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
                            crate::ui::bg::spawn(move || {
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
                                .hint_text("Add a note… (a, Enter to save)"),
                        );
                        // Focus is requested after the input is laid out, so the `a`
                        // keystroke itself isn't typed into the field this frame.
                        if key_add {
                            resp.request_focus();
                        }
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
                            crate::ui::bg::spawn(move || {
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

            // Action row reserved as a bottom strip. Declared before the scroll area so egui
            // subtracts its *true* height (no magic constant, survives reflow). Transparent
            // frame + no separator line so it reads as part of the editor, not a separate panel;
            // the manual separator below is the only divider.
            egui::Panel::bottom("editor_actions")
                .frame(egui::Frame::new())
                .show_separator_line(false)
                .show_inside(ui, |ui| {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        use crate::ui::theme::icons;
                        if common::primary_button(ui, format!("{}  Save", icons::SAVE)).clicked() {
                            submit(app);
                        }
                        if common::secondary_button(
                            ui,
                            format!("{}  Save & Done", icons::STATUS_COMPLETED),
                        )
                        .on_hover_text("Save and mark as completed (Ctrl/Cmd+Enter)")
                        .clicked()
                        {
                            app.task_manager.writer.status = TaskStatus::Completed;
                            submit(app);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if common::secondary_button(ui, format!("{}  Discard", icons::DISCARD))
                                .clicked()
                            {
                                discard(app);
                            }
                        });
                    });
                });

            // Form fills the height remaining above the action strip — no magic constant.
            egui::ScrollArea::vertical().show(ui, |ui| {
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
                        egui::RichText::new("Monospace")
                            .size(11.0)
                            .color(colors::SUBTEXT0),
                    )
                    .on_hover_text("Render details as fixed-width text");
                });
                if app.task_manager.writer.code {
                    ui.code_editor(&mut app.task_manager.writer.details_buffer);
                } else {
                    // Shift+Enter inserts a newline; plain Enter is reserved for the
                    // global submit handler below.
                    ui.add(
                        egui::TextEdit::multiline(&mut app.task_manager.writer.details_buffer)
                            .desired_width(f32::INFINITY)
                            .desired_rows(4)
                            .hint_text("Additional context…")
                            .return_key(egui::KeyboardShortcut::new(
                                egui::Modifiers::SHIFT,
                                egui::Key::Enter,
                            )),
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
                                    if common::secondary_button(
                                        ui,
                                        crate::ui::theme::icons::DISCARD,
                                    )
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
                tag_input(ui, app);

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
                {
                    use crate::ui::theme::{icons, status_color};
                    let status_label = |s: &TaskStatus| match s {
                        TaskStatus::NotStarted => {
                            format!("{}  None", icons::STATUS_NOT_STARTED)
                        }
                        TaskStatus::InProgress => {
                            format!("{}  Active", icons::STATUS_IN_PROGRESS)
                        }
                        TaskStatus::OnHold => format!("{}  Hold", icons::STATUS_ON_HOLD),
                        TaskStatus::Completed => {
                            format!("{}  Done", icons::STATUS_COMPLETED)
                        }
                    };
                    let current = app.task_manager.writer.status.clone();
                    let selected_text =
                        egui::RichText::new(status_label(&current)).color(status_color(&current));
                    egui::ComboBox::from_id_salt("status_combo")
                        .selected_text(selected_text)
                        .show_ui(ui, |ui| {
                            for status in [
                                TaskStatus::NotStarted,
                                TaskStatus::InProgress,
                                TaskStatus::OnHold,
                                TaskStatus::Completed,
                            ] {
                                let text = egui::RichText::new(status_label(&status))
                                    .color(status_color(&status));
                                ui.selectable_value(
                                    &mut app.task_manager.writer.status,
                                    status,
                                    text,
                                );
                            }
                        });
                }

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
                common::field_label(ui, "Wait until")
                    .on_hover_text("Task is hidden from the list until this date passes");
                date_field(
                    ui,
                    &mut app.task_manager.writer.wait_text,
                    &mut app.task_manager.writer.wait_until,
                    "wait_until_picker",
                    |dt| {
                        (
                            format!("hidden until {}", format_due_short(dt)),
                            colors::SUBTEXT0,
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
                        ui.selectable_value(&mut app.task_manager.writer.recurrence, None, "None");
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
        }); // end Frame

    let (enter, shift_held, cmd_held) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.modifiers.shift,
            i.modifiers.ctrl || i.modifiers.command,
        )
    });
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    // When a ComboBox or DatePicker popup is open, Enter selects the highlighted option and
    // Esc closes the popup — in both cases the key must operate the control, not the form.
    // (Esc with no focus would also make us discard the whole form.) Let egui handle it.
    let popup_open = egui::Popup::is_any_open(ui.ctx());
    // A focused button / checkbox (reached with Tab) is activated by egui on Enter or
    // Space; don't also submit the form. Text fields and "nothing focused" still save.
    let focused = ui.ctx().memory(|m| m.focused());
    let control_focused = focused.is_some() && !ui.ctx().egui_wants_keyboard_input();
    // Shift+Enter is handled by the Details TextEdit's return_key; plain Enter saves;
    // Ctrl/Cmd+Enter is Save & Done.
    if enter && !shift_held && !popup_open && !control_focused {
        if cmd_held {
            app.task_manager.writer.status = TaskStatus::Completed;
        }
        submit(app);
    }
    if esc && !popup_open {
        // Surrender focus from the active text field first; discard only when nothing
        // is focused (two-stage: first Esc blurs the field, second Esc discards).
        let focused = ui.ctx().memory(|m| m.focused());
        if let Some(id) = focused {
            ui.ctx().memory_mut(|m| m.surrender_focus(id));
        } else {
            discard(app);
        }
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

/// Tags field with a blink.cmp-style completion menu.
///
/// The menu fuzzy-matches the segment after the last comma against known tags
/// (skipping tags already typed), opens as you type with the best match
/// highlighted, and never takes focus: Tab accepts, ↓/↑ or Ctrl+N/P (Shift+Tab
/// for previous) move, Enter accepts only once you've moved, Esc / Ctrl+E closes
/// it until the text changes. With the menu closed, Tab moves to the next field.
fn tag_input(ui: &mut egui::Ui, app: &mut FastTask) {
    use crate::ui::fuzzy;
    use crate::ui::theme::colors;
    use egui::{Key, Modifiers};

    const MAX_ITEMS: usize = 8;
    let id = egui::Id::new("tag_input");
    let tm = &mut app.task_manager;

    let buf = &tm.writer.tags_buffer;
    let (head, partial) = match buf.rfind(',') {
        Some(i) => (&buf[..i], buf[i + 1..].trim()),
        None => ("", buf.trim()),
    };
    let partial = partial.to_string();
    let typed: Vec<String> = head
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    // New text → fresh menu: highlight back on the top item, un-dismiss.
    if tm.tag_menu_query != partial {
        tm.tag_menu_query = partial.clone();
        tm.tag_suggestion_idx = None;
        tm.tag_menu_dismissed = false;
    }

    let items: Vec<(String, Vec<usize>)> = if tm.tag_menu_dismissed {
        Vec::new()
    } else {
        fuzzy::rank(
            &partial,
            app.known_tags
                .iter()
                .filter(|t| !typed.contains(&t.to_lowercase())),
        )
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(t, m)| (t.clone(), m.positions))
        .collect()
    };
    let menu_open = !items.is_empty();
    if let Some(i) = tm.tag_suggestion_idx
        && i >= items.len()
    {
        tm.tag_suggestion_idx = None;
    }

    // Keys are read *before* the TextEdit is drawn and consumed, so neither the
    // field (cursor moves) nor egui's focus traversal (Tab) nor the form's
    // Enter / Esc handlers see them.
    let focused = ui.memory(|m| m.has_focus(id));
    let mut accept: Option<String> = None;
    if focused && menu_open {
        let n = items.len();
        let cur = tm.tag_suggestion_idx;
        let (next, prev, tab, enter, close) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown)
                    || i.consume_key(Modifiers::CTRL, Key::N),
                i.consume_key(Modifiers::NONE, Key::ArrowUp)
                    || i.consume_key(Modifiers::CTRL, Key::P)
                    || i.consume_key(Modifiers::SHIFT, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::Tab),
                cur.is_some() && i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape)
                    || i.consume_key(Modifiers::CTRL, Key::E),
            )
        });
        if next {
            tm.tag_suggestion_idx = Some(cur.map_or(1 % n, |c| (c + 1) % n));
        }
        if prev {
            tm.tag_suggestion_idx = Some(cur.map_or(n - 1, |c| (c + n - 1) % n));
        }
        if tab || enter {
            accept = items
                .get(tm.tag_suggestion_idx.unwrap_or(0))
                .map(|(t, _)| t.clone());
        }
        if close {
            tm.tag_menu_dismissed = true;
        }
    }

    // Mouse: clicking an item accepts it (handled below, after layout).
    let resp = ui.add(
        egui::TextEdit::singleline(&mut tm.writer.tags_buffer)
            .id(id)
            .desired_width(f32::INFINITY)
            .hint_text("work, urgent, …  (comma-separated, Tab completes)"),
    );
    // While the menu is open the field keeps Tab (accept) and Esc (close menu)
    // instead of egui moving / dropping focus at the start of the next frame.
    if menu_open && !tm.tag_menu_dismissed {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            )
        });
    }

    if menu_open && !tm.tag_menu_dismissed && accept.is_none() {
        let highlight = tm.tag_suggestion_idx.unwrap_or(0);
        egui::Frame::new()
            .fill(colors::SURFACE0)
            .stroke(egui::Stroke::new(1.0_f32, colors::SURFACE1))
            .inner_margin(egui::Margin::same(4_i8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 1.0;
                for (i, (tag, positions)) in items.iter().enumerate() {
                    let selected = i == highlight;
                    let mut job = egui::text::LayoutJob::default();
                    for (ci, ch) in tag.chars().enumerate() {
                        let hit = positions.contains(&ci);
                        job.append(
                            &ch.to_string(),
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(12.0),
                                color: if hit { colors::PEACH } else { colors::TEAL },
                                ..Default::default()
                            },
                        );
                    }
                    let fill = if selected {
                        colors::SURFACE1
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    let row = egui::Frame::new()
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(3))
                        .inner_margin(egui::Margin::symmetric(4, 1))
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            // Click-only (not FOCUSABLE) so the menu never takes focus.
                            ui.add(egui::Label::new(job).sense(egui::Sense::CLICK))
                        });
                    if row.inner.clicked() || row.response.interact(egui::Sense::CLICK).clicked() {
                        accept = Some(tag.clone());
                    }
                }
                ui.label(
                    egui::RichText::new("Tab accept · ↓↑ move · Esc close")
                        .color(colors::OVERLAY1)
                        .size(10.0),
                );
            });
    }

    if let Some(tag) = accept {
        let buf = &mut tm.writer.tags_buffer;
        let head = buf.rfind(',').map(|i| buf[..=i].to_string());
        *buf = match head {
            Some(h) => format!("{h} {tag}, "),
            None => format!("{tag}, "),
        };
        tm.tag_suggestion_idx = None;
        // Keep typing in the field, cursor at the end.
        let end = buf.chars().count();
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(end),
                )));
            state.store(ui.ctx(), id);
        }
        resp.request_focus();
    }
}
