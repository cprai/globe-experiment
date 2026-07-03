use egui_taffy::{Tui, taffy};

use super::readout::readout_block;
use super::{Instrument, leaf};
use crate::engine::ui::theme::SPACE_XXL;

/// Two labelled values side by side on one row, for compact paired readouts
/// (e.g. LAT / LON). Reuses [`super::Readout`]'s digit window (label above,
/// value + optional unit block below) for each half.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly (the
/// units default empty, so pre-unit JSON still parses); `Clone` so
/// [`crate::engine::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DualReadout {
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
    fn render(&mut self, tui: &mut Tui) {
        leaf(tui, taffy::Style::default(), |ui| {
            ui.horizontal_top(|ui| {
                readout_block(ui, &self.left_label, &self.left_value, &self.left_unit);
                // A wider-than-row gap between the pair, so the two windows
                // read as one paired instrument rather than two columns.
                ui.add_space(SPACE_XXL);
                readout_block(ui, &self.right_label, &self.right_value, &self.right_unit);
            });
        });
    }
}
