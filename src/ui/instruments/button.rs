use egui_taffy::{Tui, TuiBuilderLogic};

use super::Instrument;
use super::toggle::key_style;

/// A momentary key - the inert render data only. Drawing lives in
/// [`Button::draw`], shared by this struct's read-only [`Instrument`] impl and
/// by [`InteractiveButton`], which adds the press callback.
///
/// `Deserialize` so the `render --scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Button {
    pub label: String,
}

impl Button {
    /// Adds the key as its own grown flex node (see
    /// [`key_style`](super::toggle::key_style)); returns the response so each
    /// wrapper can read its own trigger (click vs held). Holds the widget look
    /// so the inert and interactive paths render identically. Wrapping is
    /// disabled for the same reason as the shared `leaf` helper: a
    /// longer-than-panel label (e.g. "RETROGRADE") must widen its panel, not
    /// fold onto a second line.
    fn draw(&self, tui: &mut Tui) -> egui::Response {
        tui.style(key_style())
            .wrap_mode(egui::TextWrapMode::Extend)
            .ui_add(egui::Button::new(self.label.to_uppercase()))
    }
}

impl Instrument for Button {
    fn render(&mut self, tui: &mut Tui) {
        // Inert: still clickable, but the click does nothing (e.g. a mock panel).
        let _ = self.draw(tui);
    }
}

/// A [`Button`] wired to a press callback. `on_press` fires on click. The
/// borrow `'a` is the `&mut self` of the producing
/// [`crate::ui::UIDrawable::get_drawables`], so the closure can capture a
/// disjoint mutable field of live state.
///
/// No live panel drives a click-fired button today (the burn keys hold-fire
/// via [`InteractiveHoldButton`]); this completes the control set as part of
/// the reusable instrument library, so `dead_code` is allowed until a
/// producer constructs one.
#[allow(dead_code)]
pub struct InteractiveButton<'a> {
    pub button: Button,
    pub on_press: Box<dyn FnMut() + 'a>,
}

impl Instrument for InteractiveButton<'_> {
    fn render(&mut self, tui: &mut Tui) {
        if self.button.draw(tui).clicked() {
            (self.on_press)();
        }
    }
}

/// A [`Button`] wired to a hold callback: `on_hold` fires **every frame** the
/// key is held down (e.g. a thruster burn that lasts as long as the press).
/// Frames keep coming while held because the burn only matters with the
/// simulation clock running, and a running clock already requests every
/// frame. Same borrow rules as [`InteractiveButton`].
pub struct InteractiveHoldButton<'a> {
    pub button: Button,
    pub on_hold: Box<dyn FnMut() + 'a>,
}

impl Instrument for InteractiveHoldButton<'_> {
    fn render(&mut self, tui: &mut Tui) {
        if self.button.draw(tui).is_pointer_button_down_on() {
            (self.on_hold)();
        }
    }
}
