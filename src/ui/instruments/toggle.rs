use egui::Stroke;

use super::Instrument;
use crate::ui::theme::{ACCENT_GREEN, KEY_ACTIVE};

/// A latching key that lights green while `active` - the inert render data
/// only. Drawing lives in [`Toggle::draw`], shared by this struct's read-only
/// [`Instrument`] impl and by [`InteractiveToggle`], which adds the toggle
/// callback.
///
/// `Deserialize` so the `render --scene` `ui` JSON can name it directly (the
/// `active` flag still drives its lit look); `Clone` so [`crate::ui::PanelSet`]
/// can hand a copy out of its borrowing `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toggle {
    pub position: [f32; 2],
    pub label: String,
    pub active: bool,
}

impl Toggle {
    /// Draws the latching key into `ui`; returns whether it was clicked this
    /// frame. Holds the lit-look override so the inert and interactive paths
    /// render identically.
    fn draw(&self, ui: &mut egui::Ui) -> bool {
        toggle_key(ui, &self.label, self.active).clicked()
    }
}

impl Instrument for Toggle {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        // Inert: still clickable, but the click does nothing (e.g. a mock panel).
        let _ = self.draw(ui);
    }
}

/// A [`Toggle`] wired to a toggle callback. `on_toggle` fires on click; the
/// producer owns flipping the `active` state it reflects. The borrow `'a` is
/// the `&mut self` of the producing [`crate::ui::UIDrawable::get_drawables`],
/// so the closure can capture a disjoint mutable field of live state.
pub struct InteractiveToggle<'a> {
    pub toggle: Toggle,
    pub on_toggle: Box<dyn FnMut() + 'a>,
}

impl Instrument for InteractiveToggle<'_> {
    fn position(&self) -> [f32; 2] {
        self.toggle.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        if self.toggle.draw(ui) {
            (self.on_toggle)();
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
