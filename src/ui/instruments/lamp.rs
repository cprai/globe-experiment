use egui::Stroke;

use super::Instrument;
use crate::ui::theme::{
    ACCENT_GREEN, ACCENT_RED, BEVEL_DARK, HEADER_AMBER, LABEL_DIM, RECESS_FILL,
};

/// The *semantic* condition a producer selects for a [`Lamp`], mapped to a lamp
/// color in [`Lamp::render`] (the producer never names a color). Serde
/// snake_case so the mock JSON can say `"status": "ok"`.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LampStatus {
    /// Nominal - green.
    Ok,
    /// Caution - amber.
    Caution,
    /// Fault - red.
    Fault,
    /// Unlit/inactive - dim.
    Off,
}

/// A status indicator lamp: a colored disc in a recessed socket keyed to
/// `status`, plus a dim caption.
pub struct Lamp {
    pub position: [f32; 2],
    pub label: String,
    pub status: LampStatus,
}

impl Instrument for Lamp {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        // The `status` picks the lamp color here - the producer only names the
        // condition.
        let color = match self.status {
            LampStatus::Ok => ACCENT_GREEN,
            LampStatus::Caution => HEADER_AMBER,
            LampStatus::Fault => ACCENT_RED,
            LampStatus::Off => LABEL_DIM,
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
            let center = rect.center();
            let painter = ui.painter();
            painter.circle_filled(center, 6.0, RECESS_FILL);
            // A soft halo, then the lit disc.
            painter.circle_filled(center, 5.0, color.gamma_multiply(0.35));
            painter.circle_filled(center, 3.2, color);
            painter.circle_stroke(center, 6.0, Stroke::new(1.0, BEVEL_DARK));
            ui.label(egui::RichText::new(self.label.to_uppercase()).color(LABEL_DIM));
        });
    }
}
