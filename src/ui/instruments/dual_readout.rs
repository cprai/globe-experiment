use super::Instrument;
use super::readout::readout_block;

/// Two labelled values side by side on one row, for compact paired readouts
/// (e.g. LAT / LON). Reuses [`super::Readout`]'s digit window (label above,
/// value + optional unit block below) for each half.
///
/// `Deserialize` so the `render --scene` `ui` JSON can name it directly (the
/// units default empty, so pre-unit JSON still parses); `Clone` so
/// [`crate::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DualReadout {
    pub position: [f32; 2],
    pub left_label: String,
    pub left_value: String,
    #[serde(default)]
    pub left_unit: String,
    pub right_label: String,
    pub right_value: String,
    #[serde(default)]
    pub right_unit: String,
}

impl Instrument for DualReadout {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        ui.horizontal_top(|ui| {
            readout_block(ui, &self.left_label, &self.left_value, &self.left_unit);
            ui.add_space(14.0);
            readout_block(ui, &self.right_label, &self.right_value, &self.right_unit);
        });
    }
}
