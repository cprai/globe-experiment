use egui::Stroke;

use super::Instrument;
use crate::ui::theme::{ACCENT_GREEN, KEY_ACTIVE};

/// A latching key that lights green while `active`. `on_toggle` fires on click
/// (when `Some`); the producer owns flipping the state it reflects. A `None`
/// callback renders an inert key (e.g. for a mock panel).
pub struct Toggle<'a> {
    pub position: [f32; 2],
    pub label: String,
    pub active: bool,
    pub on_toggle: Option<Box<dyn FnMut() + 'a>>,
}

impl Instrument for Toggle<'_> {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        if toggle_key(ui, &self.label, self.active).clicked()
            && let Some(callback) = self.on_toggle.as_mut()
        {
            callback();
        }
    }
}

/// Renders a latching key that lights green while `active`. When lit, every
/// pointer state (rest/hover/press) is forced to the green look so the key
/// reads as an engaged lamp, not a momentary button. The style override is
/// local to this element's child `Ui`.
fn toggle_key(ui: &mut egui::Ui, label: &str, active: bool) -> egui::Response {
    let text = egui::RichText::new(label.to_uppercase());
    if !active {
        return ui.button(text);
    }
    {
        let widgets = &mut ui.style_mut().visuals.widgets;
        for state in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            state.bg_fill = KEY_ACTIVE;
            state.weak_bg_fill = KEY_ACTIVE;
            state.bg_stroke = Stroke::new(1.0, ACCENT_GREEN);
            state.fg_stroke = Stroke::new(1.0, ACCENT_GREEN);
        }
    }
    ui.button(text.color(ACCENT_GREEN))
}
