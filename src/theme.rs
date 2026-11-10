use eframe::egui::{self, Color32, CornerRadius, Margin, RichText, Stroke, Ui, Vec2};

pub const SIDEBAR: Color32 = Color32::from_rgb(0x23, 0x26, 0x2c);
pub const SIDEBAR_SEL: Color32 = Color32::from_rgb(0x36, 0x3a, 0x43);
pub const ACCENT: Color32 = Color32::from_rgb(0x2f, 0x80, 0xed);
pub const GREEN: Color32 = Color32::from_rgb(0x2e, 0xa8, 0x62);
pub const BG: Color32 = Color32::from_rgb(0xf3, 0xf4, 0xf6);
pub const CARD: Color32 = Color32::WHITE;
pub const TEXT: Color32 = Color32::from_rgb(0x2b, 0x2e, 0x33);
pub const MUTED: Color32 = Color32::from_rgb(0x7a, 0x7f, 0x87);
pub const WARN_BG: Color32 = Color32::from_rgb(0xff, 0xf4, 0xd6);
pub const LINE: Color32 = Color32::from_rgb(0xe1, 0xe3, 0xe7);

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(8)
        .stroke(Stroke::new(1.0_f32, LINE))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

pub fn primary(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(Color32::WHITE).strong().size(15.0))
            .fill(if enabled { ACCENT } else { Color32::from_rgb(0xa8, 0xc4, 0xee) })
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(150.0, 36.0)),
    )
}

pub fn secondary(ui: &mut Ui, text: &str, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(ACCENT).strong().size(15.0))
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, ACCENT))
            .corner_radius(CornerRadius::same(5))
            .min_size(Vec2::new(130.0, 36.0)),
    )
}
