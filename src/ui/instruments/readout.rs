use egui::{CornerRadius, Margin, Stroke};

use super::Instrument;
use crate::ui::theme::{BEVEL_DARK, LABEL_DIM, READOUT_CREAM, RECESS_FILL};

/// A labelled value: a dim caption beside a cream value in a recessed readout
/// window (the label/value split that keeps a value reading as the lit
/// element).
///
/// `Deserialize` so the `render --scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Readout {
    pub position: [f32; 2],
    pub label: String,
    pub value: String,
}

impl Instrument for Readout {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        ui.horizontal(|ui| readout_pair(ui, &self.label, &self.value));
    }
}

/// Renders a dim engraved label beside its value, the value sitting in a
/// recessed cream-on-black readout window. Called inside a horizontal layout.
/// Shared with [`super::DualReadout`], which lays out two side by side.
pub(super) fn readout_pair(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label.to_uppercase()).color(LABEL_DIM));
    egui::Frame::new()
        .fill(RECESS_FILL)
        .stroke(Stroke::new(1.0, BEVEL_DARK))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(value.to_uppercase()).color(READOUT_CREAM));
        });
}
