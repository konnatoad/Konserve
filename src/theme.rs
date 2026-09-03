//! one-stop visual polish: accent colour + a light restyle of egui's defaults.
//! keeps the stock dark egui feel, just rounder, roomier and a bit less flat.
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Stroke, TextStyle,
};

/// primary accent, used for the main action buttons, progress bars and selection
pub const ACCENT: Color32 = Color32::from_rgb(60, 130, 220);
/// accent when hovered / for the drop-zone highlight
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(84, 156, 240);

/// installs the app style/visuals onto the egui context, call once at startup
pub fn install(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        // --- spacing: a touch more air between things (kept modest so the
        // fixed-size Settings tab still fits without a scrollbar) ---
        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 4.0);
        s.button_padding = egui::vec2(9.0, 4.0);
        s.interact_size.y = 22.0;
        s.indent = 16.0;
        s.menu_margin = egui::Margin::same(6);

        // --- type scale: bump Small + Heading so section labels and titles
        // read cleaner; Body stays 13 to keep the fixed-size layout intact ---
        style.text_styles = [
            (TextStyle::Small, FontId::new(10.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();

        // --- interaction: pointer cursor on clickables, like the web ---
        style.visuals.interact_cursor = Some(CursorIcon::PointingHand);

        // --- visuals: rounder corners, softer lines, accent-tinted selection ---
        let v = &mut style.visuals;
        v.window_corner_radius = CornerRadius::same(9);
        v.menu_corner_radius = CornerRadius::same(8);

        v.panel_fill = Color32::from_gray(25);
        v.faint_bg_color = Color32::from_gray(37);
        v.extreme_bg_color = Color32::from_gray(18);
        v.striped = true;

        v.selection.bg_fill = Color32::from_rgba_unmultiplied(60, 130, 220, 96);
        v.selection.stroke = Stroke::new(1.0, ACCENT_HOVER);
        v.hyperlink_color = ACCENT_HOVER;
        v.warn_fg_color = Color32::from_rgb(240, 185, 70);
        v.error_fg_color = Color32::from_rgb(238, 102, 90);

        // rounder widgets + softer separators/outlines
        let w = &mut v.widgets;
        for wv in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            wv.corner_radius = CornerRadius::same(6);
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(46));
        w.inactive.weak_bg_fill = Color32::from_gray(54);
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(110));
    });
}

/// a filled accent button that reads as the primary action on a screen
pub fn primary_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label.to_owned()).color(Color32::WHITE)).fill(ACCENT)
}
