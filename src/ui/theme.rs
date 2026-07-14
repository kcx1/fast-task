use egui::{Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Visuals};

/// Catppuccin Macchiato palette
pub mod colors {
    use egui::Color32;

    pub const BASE: Color32 = Color32::from_rgb(0x24, 0x27, 0x3A);
    pub const MANTLE: Color32 = Color32::from_rgb(0x1E, 0x20, 0x30);
    pub const SURFACE0: Color32 = Color32::from_rgb(0x36, 0x3A, 0x4F);
    pub const SURFACE1: Color32 = Color32::from_rgb(0x49, 0x4D, 0x64);
    pub const SURFACE2: Color32 = Color32::from_rgb(0x5B, 0x60, 0x78);
    pub const OVERLAY0: Color32 = Color32::from_rgb(0x6E, 0x73, 0x8D);
    pub const OVERLAY1: Color32 = Color32::from_rgb(0x80, 0x87, 0xA2);
    pub const SUBTEXT0: Color32 = Color32::from_rgb(0xA5, 0xAD, 0xCB);
    pub const TEXT: Color32 = Color32::from_rgb(0xCA, 0xD3, 0xF5);
    pub const LAVENDER: Color32 = Color32::from_rgb(0xB7, 0xBD, 0xF8);
    pub const BLUE: Color32 = Color32::from_rgb(0x8A, 0xAD, 0xF4);
    pub const TEAL: Color32 = Color32::from_rgb(0x8B, 0xD5, 0xCA);
    pub const TEAL_DIM: Color32 = Color32::from_rgb(0x5E, 0x8E, 0x87);
    pub const GREEN: Color32 = Color32::from_rgb(0xA6, 0xDA, 0x95);
    pub const YELLOW: Color32 = Color32::from_rgb(0xEE, 0xD4, 0x9F);
    pub const PEACH: Color32 = Color32::from_rgb(0xF5, 0xA9, 0x7F);
    pub const RED: Color32 = Color32::from_rgb(0xED, 0x87, 0x96);
    pub const MAUVE: Color32 = Color32::from_rgb(0xC6, 0xA0, 0xF6);
}

const ROUND: CornerRadius = CornerRadius {
    nw: 4,
    ne: 4,
    sw: 4,
    se: 4,
};

pub mod icons {
    pub const STATUS_NOT_STARTED: &str = "\u{F0766}"; // nf-md-circle_outline
    pub const STATUS_IN_PROGRESS: &str = "\u{F0765}"; // nf-md-circle_half_full
    pub const STATUS_ON_HOLD: &str = "\u{F05E0}"; // nf-md-pause_circle_outline
    pub const STATUS_COMPLETED: &str = "\u{F0133}"; // nf-md-check_circle_outline

    pub const PRIORITY_URGENT: &str = "\u{F06A2}"; // nf-md-alert_circle
    pub const PRIORITY_LOW: &str = "\u{F004A}"; // nf-md-arrow_down_bold_circle

    pub const SAVE: &str = "\u{F0818}"; // nf-md-content_save
    pub const DISCARD: &str = "\u{F0156}"; // nf-md-close_circle
    pub const DELETE: &str = "\u{F01B4}"; // nf-md-delete
    pub const NEW: &str = "\u{F0417}"; // nf-md-plus_circle

    pub const MODE_NORMAL: &str = "\u{F071E}"; // nf-md-circle_medium
    pub const MODE_INSERT: &str = "\u{F03EB}"; // nf-md-pencil
    pub const MODE_VISUAL: &str = "\u{F0208}"; // nf-md-eye

    pub const PIN: &str = "\u{F0231}"; // nf-md-pin
}

/// Apply the Catppuccin Macchiato theme. Call once on app startup.
pub fn apply(ctx: &egui::Context) {
    // Load bundled Nerd Font symbols as fallback for icon codepoints
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "SymbolsNerdFont".to_owned(),
        egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/SymbolsNerdFontMono-Regular.ttf"
        ))
        .into(),
    );
    fonts
        .families
        .get_mut(&egui::FontFamily::Proportional)
        .unwrap()
        .push("SymbolsNerdFont".to_owned());
    fonts
        .families
        .get_mut(&egui::FontFamily::Monospace)
        .unwrap()
        .push("SymbolsNerdFont".to_owned());
    ctx.set_fonts(fonts);
    let mut visuals = Visuals::dark();

    // Backgrounds
    visuals.window_fill = colors::BASE;
    visuals.panel_fill = colors::BASE;
    visuals.faint_bg_color = colors::MANTLE; // table stripe rows
    visuals.extreme_bg_color = colors::MANTLE; // text input backgrounds

    // Window chrome
    visuals.window_stroke = Stroke::new(1.0_f32, colors::SURFACE1);
    visuals.window_shadow = Shadow::NONE;

    // Widgets
    {
        let w = &mut visuals.widgets;

        w.noninteractive.weak_bg_fill = colors::BASE;
        w.noninteractive.bg_fill = colors::SURFACE0;
        w.noninteractive.bg_stroke = Stroke::new(0.5_f32, colors::SURFACE2);
        w.noninteractive.fg_stroke = Stroke::new(1.0_f32, colors::SUBTEXT0);
        w.noninteractive.corner_radius = ROUND;

        w.inactive.weak_bg_fill = colors::SURFACE0;
        w.inactive.bg_fill = colors::SURFACE0;
        w.inactive.bg_stroke = Stroke::new(1.0_f32, colors::SURFACE1);
        w.inactive.fg_stroke = Stroke::new(1.0_f32, colors::TEXT);
        w.inactive.corner_radius = ROUND;

        w.hovered.weak_bg_fill = colors::SURFACE1;
        w.hovered.bg_fill = colors::SURFACE1;
        w.hovered.bg_stroke = Stroke::new(1.0_f32, colors::BLUE);
        w.hovered.fg_stroke = Stroke::new(1.5_f32, colors::TEXT);
        w.hovered.corner_radius = ROUND;

        w.active.weak_bg_fill = colors::SURFACE2;
        w.active.bg_fill = colors::SURFACE2;
        w.active.bg_stroke = Stroke::new(1.0_f32, colors::BLUE);
        w.active.fg_stroke = Stroke::new(2.0_f32, colors::TEXT);
        w.active.corner_radius = ROUND;

        w.open.weak_bg_fill = colors::SURFACE1;
        w.open.bg_fill = colors::SURFACE1;
        w.open.bg_stroke = Stroke::new(1.0_f32, colors::BLUE);
        w.open.fg_stroke = Stroke::new(1.0_f32, colors::TEXT);
        w.open.corner_radius = ROUND;
    }

    // Selection highlight
    visuals.selection.bg_fill = Color32::from_rgba_premultiplied(0x8A, 0xAD, 0xF4, 55);
    visuals.selection.stroke = Stroke::new(1.0_f32, colors::BLUE);

    // Misc
    visuals.override_text_color = Some(colors::TEXT);
    visuals.hyperlink_color = colors::BLUE;
    visuals.error_fg_color = colors::RED;
    visuals.warn_fg_color = colors::YELLOW;
    visuals.code_bg_color = colors::MANTLE;

    ctx.set_visuals(visuals);

    // Typography and spacing
    let mut style = (*ctx.global_style()).clone();

    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(6.0_f32, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(3.0_f32, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(3.0_f32, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(1.0_f32, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(2.0_f32, FontFamily::Monospace),
        ),
    ]
    .into();

    style.spacing.item_spacing = egui::vec2(6.0_f32, 4.0);
    style.spacing.button_padding = egui::vec2(0.0_f32, 5.0);
    style.spacing.window_margin = egui::Margin::same(12_i8);
    style.spacing.indent = 16.0_f32;

    ctx.set_global_style(style);
}

/// Color for a task status indicator.
pub fn status_color(status: &crate::database::models::TaskStatus) -> Color32 {
    use crate::database::models::TaskStatus;
    match status {
        TaskStatus::NotStarted => colors::SUBTEXT0,
        TaskStatus::InProgress => colors::BLUE,
        TaskStatus::Completed => colors::GREEN,
        TaskStatus::OnHold => colors::YELLOW,
    }
}

/// Color for a priority indicator.
pub fn priority_color(priority: &crate::database::models::Priority) -> Color32 {
    use crate::database::models::Priority;
    match priority {
        Priority::Urgent => colors::RED,
        Priority::Normal => colors::TEXT,
        Priority::Low => colors::SUBTEXT0,
    }
}
