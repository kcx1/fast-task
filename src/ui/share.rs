//! Desktop side of the LAN share: starting / stopping it, the link + QR popup,
//! kept links, and the activity side panel (`Shift+M`).

use eframe::egui;

use crate::database::database::ShareSettings;
use crate::database::models::ActivityEntry;
use crate::local_share::server::{self, ShareConfig};
use crate::ui::app::{DB, FastTask, UpdateMessage};
use crate::ui::theme::{colors, icons};
use crate::ui::widgets::common;
use crate::ui::widgets::errors::{ErrorSeverity, ErrorUi};

/// How many log lines the side panel shows.
const PANEL_ENTRIES: usize = 200;

/// Share state that isn't the server itself.
#[derive(Default)]
pub struct ShareUi {
    /// Saved preferences; loaded on first use.
    prefs: Option<ShareSettings>,
    /// Activity side panel is open.
    pub show_activity: bool,
    pub activity: Vec<ActivityEntry>,
    /// Watches DB writes while the panel is open, to refetch the log.
    activity_changes: Option<tokio::sync::watch::Receiver<u64>>,
    /// The popup's "your name" field while it's open; saved when it loses
    /// focus or the popup closes, not per keystroke.
    host_name_buf: Option<String>,
    /// The popup's settings column (gear) is open.
    pub(crate) show_settings: bool,
}

/// Side of the popup's QR code; the buttons under it match its width. It
/// shrinks toward `QR_MIN` to make room for the settings column.
const QR_SIZE: f32 = 220.0;
const QR_MIN: f32 = 160.0;
/// The popup's settings column, side by side with the QR code.
const SETTINGS_WIDTH: f32 = 190.0;
const SETTINGS_MIN: f32 = 150.0;
/// Inner padding of the popup.
const PADDING: i8 = 18;
/// Space kept between the popup and the window edge.
const SCREEN_MARGIN: f32 = 12.0;
/// Widest a hover tip gets (narrower windows wrap sooner).
const TIP_WIDTH: f32 = 260.0;

/// Sizes for the share popup's content, fitted to `room` points of width.
#[derive(Debug, PartialEq)]
struct PopupLayout {
    qr: f32,
    settings: f32,
    /// Settings beside the QR code; otherwise stacked under the buttons.
    side_by_side: bool,
    /// Total content width.
    width: f32,
}

impl PopupLayout {
    fn fit(room: f32, gap: f32, show_settings: bool) -> Self {
        // A separator between columns: a gap each side of a 1pt line.
        let divider = 2.0 * gap + 1.0;
        if show_settings && room >= QR_MIN + divider + SETTINGS_MIN {
            // Settings take what's left after the QR code, which gives up
            // space first (down to QR_MIN) before the settings shrink.
            let settings = (room - divider - QR_MIN).min(SETTINGS_WIDTH);
            let qr = (room - divider - settings).min(QR_SIZE);
            return Self {
                qr,
                settings,
                side_by_side: true,
                width: qr + divider + settings,
            };
        }
        let qr = room.clamp(QR_MIN.min(room.max(0.0)), QR_SIZE);
        Self {
            qr,
            settings: qr,
            side_by_side: false,
            width: qr,
        }
    }
}

/// A small, wrapping hover tip — the default is body-sized and wide.
fn tip(response: egui::Response, text: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(TIP_WIDTH.min(ui.ctx().content_rect().width() - 2.0 * SCREEN_MARGIN));
        ui.label(egui::RichText::new(text).size(11.0));
    })
}

/// Hover tip showing the share URL, small and broken across lines anywhere
/// (it has no spaces to wrap at).
fn url_tip(response: egui::Response, url: &str) -> egui::Response {
    response.on_hover_ui(|ui| {
        let width = TIP_WIDTH.min(ui.ctx().content_rect().width() - 2.0 * SCREEN_MARGIN);
        ui.set_max_width(width);
        let mut job = egui::text::LayoutJob::single_section(
            url.to_string(),
            egui::TextFormat::simple(egui::FontId::monospace(10.0), colors::SUBTEXT0),
        );
        job.wrap.max_width = width;
        job.wrap.break_anywhere = true;
        ui.label(job);
    })
}

/// Name, allow edits, keep link, and what they mean. Returns true when the name
/// field lost focus (time to save it).
fn settings_column(
    ui: &mut egui::Ui,
    host_name: &mut String,
    allow_edits: &mut bool,
    keep_link: &mut bool,
) -> bool {
    common::field_label(ui, "Your name");
    let commit = tip(
        ui.add(
            egui::TextEdit::singleline(host_name)
                .id(egui::Id::new("share_host_name"))
                .char_limit(crate::local_share::api::NAME_MAX)
                .desired_width(f32::INFINITY),
        ),
        "How browsers see your changes in the activity log",
    )
    .lost_focus();
    ui.add_space(6.0);
    tip(
        ui.checkbox(allow_edits, "Allow edits"),
        "Lets browsers add, edit, complete and delete tasks and notes. \
         Undo (u) works on their changes too.",
    );
    tip(
        ui.checkbox(keep_link, "Keep link after restarts"),
        "Reuse this link (and the edit setting) every time you share, so people \
         you gave it to keep access. The refresh button replaces it. The address \
         can still change if your computer gets a new IP on the network.",
    );
    ui.add_space(6.0);
    let (warning, color) = if *allow_edits {
        (
            "Anyone with the link on your network can view and edit your tasks.",
            colors::PEACH,
        )
    } else {
        (
            "Anyone with the link on your network can view your tasks.",
            colors::OVERLAY0,
        )
    };
    ui.label(egui::RichText::new(warning).color(color).size(11.0));
    commit
}

/// The server config for the next start: a kept link reuses its token, port
/// and edit setting; otherwise a fresh, read-only link.
fn config_for(prefs: &ShareSettings, host_name: String) -> ShareConfig {
    let kept = prefs.keep_link && prefs.token.is_some();
    ShareConfig {
        token: if kept { prefs.token.clone() } else { None },
        port: if kept { prefs.port } else { None },
        allow_edits: kept && prefs.allow_edits,
        host_name,
    }
}

/// `prefs` updated from the running share: with "keep link" its token, port and
/// edit setting are remembered; without, none are.
fn remember(mut prefs: ShareSettings, running: Option<(String, u16, bool)>) -> ShareSettings {
    if !prefs.keep_link {
        prefs.token = None;
        prefs.port = None;
        prefs.allow_edits = false;
    } else if let Some((token, port, allow_edits)) = running {
        prefs.token = Some(token);
        prefs.port = Some(port);
        prefs.allow_edits = allow_edits;
    }
    prefs
}

/// `$USER`, capitalized — how browsers see your edits until you rename it.
fn default_host_name() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let mut chars = user.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Host".into(),
    }
}

impl FastTask {
    fn share_prefs(&mut self) -> &mut ShareSettings {
        self.share_ui.prefs.get_or_insert_with(|| {
            let mut prefs = DB.share_settings();
            prefs.host_name.get_or_insert_with(default_host_name);
            prefs
        })
    }

    fn host_name(&mut self) -> String {
        self.share_prefs()
            .host_name
            .clone()
            .unwrap_or_else(default_host_name)
    }

    /// Persist the preferences, and with "keep link" the running share's token,
    /// port and edit setting, so the next start reuses the same link.
    fn save_share_prefs(&mut self) {
        let running = self
            .share
            .as_ref()
            .map(|s| (s.token().to_string(), s.port(), s.allow_edits()));
        let prefs = remember(self.share_prefs().clone(), running);
        self.share_ui.prefs = Some(prefs.clone());
        if let Err(e) = DB.save_share_settings(&prefs) {
            ErrorUi::push(
                &mut self.err_ui,
                e.context("Couldn't save the share settings"),
                ErrorSeverity::NonFatal,
            );
        }
    }

    /// `Shift+W` / the globe button: start the LAN share, or — when it's already
    /// running — reopen the popup with its link and QR code. Either way the popup
    /// ends up open; stopping is a deliberate step from there.
    pub(crate) fn share(&mut self, ctx: &egui::Context) {
        if self.share.is_some() {
            self.app_state.show_share = true;
            return;
        }
        let prefs = self.share_prefs().clone();
        let kept = prefs.keep_link && prefs.token.is_some();
        let config = config_for(&prefs, self.host_name());
        if self.start_share(ctx, config) {
            let port = self.share.as_ref().map(|s| s.port());
            if kept && prefs.port.is_some() && prefs.port != port {
                self.set_status(format!(
                    "Port {} was busy, so the link changed — share the new one",
                    prefs.port.unwrap_or_default()
                ));
            }
        }
    }

    /// Start the server; on success opens the popup and turns on the activity
    /// log. Returns whether it started.
    fn start_share(&mut self, ctx: &egui::Context, config: ShareConfig) -> bool {
        let (tx, ctx) = (self.backend_manager.tx.clone(), ctx.clone());
        let on_write: server::OnWrite = std::sync::Arc::new(move || {
            let _ = tx.send(UpdateMessage::RemoteChange);
            ctx.request_repaint();
        });
        match server::start(DB.clone(), config, on_write) {
            Ok(handle) => {
                self.share = Some(handle);
                self.app_state.show_share = true;
                DB.set_activity_logging(true);
                self.save_share_prefs();
                true
            }
            Err(e) => {
                ErrorUi::push(
                    &mut self.err_ui,
                    e.context("Couldn't start the local share"),
                    ErrorSeverity::NonFatal,
                );
                false
            }
        }
    }

    /// Stop the LAN share; open browsers lose the connection. A kept link
    /// works again the next time you share.
    pub(crate) fn stop_share(&mut self) {
        // Dropping the handle stops the server.
        self.share = None;
        DB.set_activity_logging(false);
        self.app_state.show_share = false;
        self.set_status("Stopped sharing".into());
    }

    /// Replace the link: same port and edit setting, new token. Anyone holding
    /// the old link loses access.
    fn new_link(&mut self, ctx: &egui::Context) {
        let Some(old) = self.share.take() else {
            return;
        };
        let config = ShareConfig {
            token: None,
            port: Some(old.port()),
            allow_edits: old.allow_edits(),
            host_name: self.host_name(),
        };
        drop(old); // frees the port for the new server
        if self.start_share(ctx, config) {
            self.set_status("New link — the old one no longer works".into());
        } else {
            DB.set_activity_logging(false);
        }
    }

    fn set_status(&mut self, msg: String) {
        self.app_state.status_msg = Some((msg, std::time::Instant::now()));
    }

    /// Share link + QR code. Esc or Close hides it; sharing keeps running.
    pub(crate) fn share_popup(&mut self, ui: &mut egui::Ui) {
        if self.share.is_none() {
            self.app_state.show_share = false;
            return;
        }
        let prefs = self.share_prefs().clone();
        let Some(share) = &self.share else {
            return;
        };
        let url = share.url().to_string();
        let mut allow_edits = share.allow_edits();
        let mut keep_link = prefs.keep_link;
        let mut host_name = self
            .share_ui
            .host_name_buf
            .take()
            .unwrap_or_else(|| prefs.host_name.clone().unwrap_or_default());
        let mut commit_name = false;
        let (mut close, mut stop, mut renew) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                i.consume_key(egui::Modifiers::SHIFT, egui::Key::W),
                false,
            )
        });
        let show_settings = self.share_ui.show_settings;
        let mut toggle_settings = false;
        let gap = ui.spacing().item_spacing.x;
        let layout = PopupLayout::fit(
            ui.ctx().content_rect().width() - 2.0 * (PADDING as f32 + SCREEN_MARGIN),
            gap,
            show_settings,
        );
        let frame = egui::Frame::popup(ui.style()).inner_margin(PADDING);
        let modal = egui::Modal::new(egui::Id::new("share_popup"))
            .frame(frame)
            .show(ui.ctx(), |ui| {
                ui.set_width(layout.width);

                // Title, with the settings gear and close ✕ on the right.
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Sharing")
                            .color(colors::LAVENDER)
                            .size(14.0)
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= tip(
                            ui.add(egui::Button::new(icons::DISCARD).frame(false)),
                            "Close (Esc) — sharing keeps running",
                        )
                        .clicked();
                        toggle_settings = tip(
                            ui.selectable_label(
                                show_settings,
                                crate::ui::view::selectable_icon(icons::SETTINGS, show_settings),
                            ),
                            "Settings: your name, edits, keep link",
                        )
                        .clicked();
                    });
                });
                ui.separator();
                ui.add_space(4.0);

                let qr = layout.qr;
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(qr);
                        url_tip(paint_qr(ui, &url, qr), &url);
                        ui.add_space(8.0);
                        // Copy link ¾, new link ¼.
                        let row_height = 28.0;
                        let copy_width = (qr - gap) * 0.75;
                        ui.horizontal(|ui| {
                            let copy = egui::Button::new(
                                egui::RichText::new("Copy link")
                                    .color(colors::MANTLE)
                                    .strong(),
                            )
                            .fill(colors::BLUE)
                            .stroke(egui::Stroke::NONE);
                            if url_tip(ui.add_sized([copy_width, row_height], copy), &url).clicked()
                            {
                                ui.ctx().copy_text(url.clone());
                                self.app_state.status_msg =
                                    Some(("Share link copied".into(), std::time::Instant::now()));
                            }
                            renew = tip(
                                ui.add_sized(
                                    [qr - gap - copy_width, row_height],
                                    egui::Button::new(icons::REFRESH),
                                ),
                                "New link — the current one stops working",
                            )
                            .clicked();
                        });
                        let stop_button = egui::Button::new(
                            egui::RichText::new("Stop sharing")
                                .color(colors::MANTLE)
                                .strong(),
                        )
                        .fill(colors::RED)
                        .stroke(egui::Stroke::NONE);
                        stop |= tip(
                            ui.add_sized([qr, row_height], stop_button),
                            "Stop sharing (Shift+W). A kept link works again next time.",
                        )
                        .clicked();

                        if show_settings && !layout.side_by_side {
                            ui.add_space(6.0);
                            ui.separator();
                            commit_name = settings_column(
                                ui,
                                &mut host_name,
                                &mut allow_edits,
                                &mut keep_link,
                            );
                        }
                    });

                    if show_settings && layout.side_by_side {
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.set_width(layout.settings);
                            commit_name = settings_column(
                                ui,
                                &mut host_name,
                                &mut allow_edits,
                                &mut keep_link,
                            );
                        });
                    }
                });
            });
        if toggle_settings {
            self.share_ui.show_settings = !show_settings;
        }

        let mut save = false;
        if allow_edits != share.allow_edits() {
            share.set_allow_edits(allow_edits);
            save = true;
        }
        let closing = modal.should_close() || close || stop || renew;
        if commit_name || closing {
            let name = host_name.trim().to_string();
            if !name.is_empty() && Some(&name) != prefs.host_name.as_ref() {
                share.set_host_name(&name);
                self.share_prefs().host_name = Some(name);
                save = true;
            }
        } else {
            // Keep typing (spaces, an emptied field) across frames.
            self.share_ui.host_name_buf = Some(host_name);
        }
        if keep_link != prefs.keep_link {
            self.share_prefs().keep_link = keep_link;
            save = true;
        }
        if save {
            self.save_share_prefs();
        }
        if closing {
            self.app_state.show_share = false;
        }
        if renew {
            let ctx = ui.ctx().clone();
            self.new_link(&ctx);
        }
        if stop {
            self.stop_share();
        }
    }

    /// `Shift+M` / the top-bar button.
    pub(crate) fn toggle_activity(&mut self) {
        let ui = &mut self.share_ui;
        ui.show_activity = !ui.show_activity;
        ui.activity_changes = None; // (re)subscribed and fetched on the next frame
    }

    /// Activity side panel. Must run before the central pane is laid out.
    pub(crate) fn activity_panel(&mut self, ui: &mut egui::Ui) {
        if !self.share_ui.show_activity {
            return;
        }
        // Refetch on open and after every write while open.
        let stale = match &mut self.share_ui.activity_changes {
            None => {
                self.share_ui.activity_changes = Some(DB.subscribe_changes());
                true
            }
            Some(rx) => {
                rx.has_changed().unwrap_or(false) && {
                    rx.mark_unchanged();
                    true
                }
            }
        };
        if stale {
            let tx = self.backend_manager.tx.clone();
            crate::ui::bg::spawn(move || match DB.activity(PANEL_ENTRIES) {
                Ok(entries) => {
                    let _ = tx.send(UpdateMessage::Activity(entries));
                }
                Err(e) => {
                    let _ = tx.send(UpdateMessage::Error(e));
                }
            });
        }

        let (mut close, mut clear) = (false, false);
        egui::Panel::right("activity_panel")
            .resizable(true)
            .default_size(260.0)
            .min_size(180.0)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    common::heading(ui, "Activity");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui
                            .small_button(icons::DISCARD)
                            .on_hover_text("Hide (Shift+M)")
                            .clicked();
                        clear = !self.share_ui.activity.is_empty()
                            && ui
                                .small_button("Clear")
                                .on_hover_text("Delete the whole activity log")
                                .clicked();
                    });
                });
                ui.separator();
                if self.share_ui.activity.is_empty() {
                    let hint = if self.share.is_some() {
                        "No changes yet. Edits made while sharing — yours and from browsers — show up here."
                    } else {
                        "Edits made while sharing show up here. Start sharing with Shift+W."
                    };
                    ui.label(
                        egui::RichText::new(hint)
                            .color(colors::OVERLAY1)
                            .size(11.0)
                            .italics(),
                    );
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("activity_scroll")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        crate::ui::view::activity_list(ui, &self.share_ui.activity, |who| {
                            match who {
                                None => ("You".into(), colors::BLUE),
                                Some(name) => (name.clone(), crate::ui::view::person_color(name)),
                            }
                        });
                    });
            });
        if close {
            self.toggle_activity();
        }
        if clear {
            let tx = self.backend_manager.tx.clone();
            crate::ui::bg::spawn(move || {
                if let Err(e) = DB.clear_activity() {
                    let _ = tx.send(UpdateMessage::Error(e));
                }
            });
        }
    }
}

/// Draw `text` as a QR code `size` points square (dark on white, with the
/// standard 4-module quiet zone so phone cameras lock on).
fn paint_qr(ui: &mut egui::Ui, text: &str, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return response;
    };
    let width = code.width();
    const QUIET: usize = 4;
    let module = size / (width + 2 * QUIET) as f32;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, egui::Color32::WHITE);
    for (i, color) in code.to_colors().into_iter().enumerate() {
        if color == qrcode::Color::Dark {
            let (x, y) = (i % width + QUIET, i / width + QUIET);
            let min = rect.min + egui::vec2(x as f32 * module, y as f32 * module);
            painter.rect_filled(
                egui::Rect::from_min_size(min, egui::vec2(module, module)),
                0.0,
                egui::Color32::BLACK,
            );
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_fits_the_window() {
        let gap = 6.0;
        // Wide: full-size QR and settings side by side.
        let wide = PopupLayout::fit(800.0, gap, true);
        assert_eq!(
            (wide.qr, wide.settings, wide.side_by_side),
            (QR_SIZE, SETTINGS_WIDTH, true)
        );
        // A bit narrower: the QR code gives up space first.
        let snug = PopupLayout::fit(380.0, gap, true);
        assert!(snug.side_by_side && snug.qr < QR_SIZE && snug.settings == SETTINGS_WIDTH);
        assert!(snug.width <= 380.0);
        // Too narrow for two columns: settings stack under the buttons.
        let narrow = PopupLayout::fit(300.0, gap, true);
        assert!(!narrow.side_by_side && narrow.width <= 300.0);
        // Never wider than the room, settings or not.
        for room in [120.0, 200.0, 330.0, 420.0, 1000.0] {
            for settings in [false, true] {
                let l = PopupLayout::fit(room, gap, settings);
                assert!(
                    l.width <= room.max(QR_MIN.min(room)) + 0.01,
                    "{room} {settings}: {l:?}"
                );
            }
        }
    }

    fn kept() -> ShareSettings {
        ShareSettings {
            keep_link: true,
            token: Some("abc".into()),
            port: Some(9000),
            allow_edits: true,
            host_name: Some("Casey".into()),
        }
    }

    #[test]
    fn a_kept_link_restarts_with_the_same_token_port_and_edits() {
        let config = config_for(&kept(), "Casey".into());
        assert_eq!(config.token.as_deref(), Some("abc"));
        assert_eq!(config.port, Some(9000));
        assert!(config.allow_edits);
    }

    #[test]
    fn without_keep_link_every_start_is_fresh_and_read_only() {
        let prefs = ShareSettings {
            keep_link: false,
            ..kept()
        };
        let config = config_for(&prefs, "Casey".into());
        assert_eq!(
            (config.token, config.port, config.allow_edits),
            (None, None, false)
        );
        // And nothing about the link is saved.
        let saved = remember(prefs, Some(("xyz".into(), 8080, true)));
        assert_eq!(
            (saved.token, saved.port, saved.allow_edits),
            (None, None, false)
        );
    }

    #[test]
    fn a_new_link_replaces_the_saved_token() {
        let saved = remember(kept(), Some(("new".into(), 9000, false)));
        assert_eq!(saved.token.as_deref(), Some("new"));
        assert!(!saved.allow_edits);
        assert_eq!(
            config_for(&saved, "Casey".into()).token.as_deref(),
            Some("new")
        );
    }
}
