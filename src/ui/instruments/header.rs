use egui::Stroke;

use super::Instrument;
use crate::ui::theme::{BEVEL_LIGHT, HEADER_AMBER};

/// A section header: a bold amber title with a rule ruled across the panel
/// width beneath it - the labelled divider that tops each cluster on the Apollo
/// panels.
///
/// `Deserialize` so the `render --scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub position: [f32; 2],
    pub title: String,
}

impl Instrument for Header {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, child_rect: egui::Rect, panel_size: egui::Vec2) {
        // The rule spans the full panel width (hence `panel_size`), under the
        // title baseline.
        let rule_y = child_rect.top() + 19.0;
        ui.painter().hline(
            child_rect.left()..=child_rect.left() + panel_size.x,
            rule_y,
            Stroke::new(1.0, BEVEL_LIGHT),
        );
        ui.label(
            egui::RichText::new(self.title.to_uppercase())
                .color(HEADER_AMBER)
                .strong()
                .size(15.0),
        );
    }
}
