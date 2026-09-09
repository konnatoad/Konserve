//! one-stop visual polish: accent colour + a black-surface restyle of egui's
//! defaults — rounder, roomier, with a raised-card / sunken-field hierarchy.
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Stroke, TextStyle,
};

/// primary accent, used for the main action buttons, progress bars and selection
pub const ACCENT: Color32 = Color32::from_rgb(60, 130, 220);
/// accent when hovered / for the drop-zone highlight
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(84, 156, 240);

/// app background — essentially black
pub const BG: Color32 = Color32::from_gray(6);
/// raised surfaces: cards, the tab-bar container
pub const CARD: Color32 = Color32::from_gray(19);
/// sunken surfaces: text fields, the status box, the drop zone
pub const SUNKEN: Color32 = Color32::BLACK;

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

        // --- visuals: black surfaces, rounder corners, accent-tinted selection ---
        let v = &mut style.visuals;
        v.window_corner_radius = CornerRadius::same(9);
        v.menu_corner_radius = CornerRadius::same(8);

        v.panel_fill = BG;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, Color32::from_gray(40));
        v.faint_bg_color = CARD;
        v.extreme_bg_color = SUNKEN;
        v.code_bg_color = SUNKEN;
        v.striped = true;

        v.selection.bg_fill = Color32::from_rgba_unmultiplied(60, 130, 220, 96);
        v.selection.stroke = Stroke::new(1.0, ACCENT_HOVER);
        v.hyperlink_color = ACCENT_HOVER;
        v.warn_fg_color = Color32::from_rgb(240, 185, 70);
        v.error_fg_color = Color32::from_rgb(238, 102, 90);

        // rounder widgets + hairline outlines that still read against black
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
        w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(40));
        w.inactive.bg_fill = Color32::from_gray(36);
        w.inactive.weak_bg_fill = Color32::from_gray(34);
        w.hovered.weak_bg_fill = Color32::from_gray(48);
        w.hovered.bg_fill = Color32::from_gray(48);
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(90));
        w.active.weak_bg_fill = Color32::from_gray(60);
        w.open.weak_bg_fill = Color32::from_gray(30);
    });
}

/// a filled accent button that reads as the primary action on a screen
pub fn primary_button(label: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label.to_owned()).color(Color32::WHITE)).fill(ACCENT)
}

/// animated pill toggle + label, a drop-in modern replacement for `ui.checkbox`.
/// the whole row is clickable and the knob slides between states.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    ui.horizontal(|ui| {
        let mut response = toggle_switch(ui, on);
        let label_resp = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
        if label_resp.clicked() {
            *on = !*on;
            response.mark_changed();
        }
        response
    })
    .inner
}

/// the bare switch part of [`toggle`]
fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let height = ui.spacing().interact_size.y.max(16.0) * 0.82;
    let size = egui::vec2(height * 1.85, height);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });

    if ui.is_rect_visible(rect) {
        // 0.0 = off, 1.0 = on, eased over the last few frames
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let radius = 0.5 * rect.height();

        let mut off_fill = ui.visuals().widgets.inactive.bg_fill;
        if response.hovered() {
            off_fill = off_fill.lerp_to_gamma(Color32::WHITE, 0.06);
        }
        let track = off_fill.lerp_to_gamma(ACCENT, t);
        ui.painter()
            .rect(rect, radius, track, Stroke::NONE, egui::StrokeKind::Inside);

        let knob_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), t);
        ui.painter().circle_filled(
            egui::pos2(knob_x, rect.center().y),
            radius - 2.5,
            Color32::WHITE,
        );
    }
    response
}
