use egui::Stroke;
use egui_taffy::{Tui, taffy};

use super::{Instrument, leaf};
use crate::ui::theme::{BEVEL_LIGHT, FONT_TITLE, HAIRLINE, HEADER_AMBER, SPACE_SM, SPACE_XS};

/// A section header: a bold amber title with a rule ruled across the panel
/// width beneath it - the labelled divider that tops each cluster on the Apollo
/// panels.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Header {
    pub title: String,
}

impl Instrument for Header {
    fn render(&mut self, tui: &mut Tui) {
        // Grow across the row so the rule spans the full panel width.
        let style = taffy::Style {
            flex_grow: 1.0,
            ..Default::default()
        };
        leaf(tui, style, |ui| {
            let title = ui.label(
                egui::RichText::new(self.title.to_uppercase())
                    .color(HEADER_AMBER)
                    .strong()
                    .size(FONT_TITLE),
            );
            // Reserve the rule's strip below the title so the node's height
            // includes it; the rule itself spans the node's full (row-grown)
            // width, not just the title's.
            ui.add_space(SPACE_SM);
            ui.painter().hline(
                ui.max_rect().x_range(),
                title.rect.bottom() + SPACE_XS,
                Stroke::new(HAIRLINE, BEVEL_LIGHT),
            );
        });
    }
}
