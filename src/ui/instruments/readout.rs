use egui::{CornerRadius, Margin, Stroke};

use super::Instrument;
use crate::ui::theme::{BEVEL_LIGHT, LABEL_DIM, READOUT_CREAM, RECESS_FILL};

/// A labelled value: a dim caption above a large cream value in a recessed
/// digit window, with an optional unit stamped as an inverted block at the
/// window's end - the DSKY-style readout from the game-UI reference (e.g.
/// `007.8|KM|`), sized so a value reads at a glance over the scene.
///
/// `Deserialize` so the `render --scene` `ui` JSON can name it directly (the
/// `unit` defaults empty, so pre-unit JSON still parses); `Clone` so
/// [`crate::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Readout {
    pub position: [f32; 2],
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub unit: String,
}

impl Instrument for Readout {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        readout_block(ui, &self.label, &self.value, &self.unit);
    }
}

/// Renders one digit-window readout: the dim engraved caption above, then the
/// recessed window holding the large cream value and, when `unit` is
/// non-empty, the inverted unit block flush at the window's end. Shared with
/// [`super::DualReadout`], which lays out two side by side.
pub(super) fn readout_block(ui: &mut egui::Ui, label: &str, value: &str, unit: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(
            egui::RichText::new(label.to_uppercase())
                .color(LABEL_DIM)
                .size(11.0),
        );
        egui::Frame::new()
            .fill(RECESS_FILL)
            // The panel body and the recessed fill composite to nearly the
            // same on-screen value, so the window outline - not the fill - is
            // what makes the digit window read as a cut-in field.
            .stroke(Stroke::new(1.0, BEVEL_LIGHT))
            .corner_radius(CornerRadius::same(2))
            // A slimmer right margin when a unit block caps the window, so the
            // block sits flush at the end like a stamped suffix.
            .inner_margin(Margin {
                left: 7,
                right: if unit.is_empty() { 7 } else { 3 },
                top: 3,
                bottom: 3,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(
                        egui::RichText::new(value.to_uppercase())
                            .color(READOUT_CREAM)
                            .size(17.0),
                    );
                    if !unit.is_empty() {
                        egui::Frame::new()
                            .fill(READOUT_CREAM)
                            .corner_radius(CornerRadius::same(1))
                            .inner_margin(Margin::symmetric(3, 1))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(unit.to_uppercase())
                                        .color(RECESS_FILL)
                                        .strong()
                                        .size(11.0),
                                );
                            });
                    }
                });
            });
    });
}
