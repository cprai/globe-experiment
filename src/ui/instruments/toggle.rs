use egui::Stroke;
use egui_taffy::{Tui, TuiBuilderLogic, taffy};

use super::Instrument;
use crate::ui::theme::{ACCENT_GREEN, HAIRLINE, KEY_LIT, KEY_LIT_TEXT};

/// A latching key that lights green while `active` - the inert render data
/// only. Drawing lives in [`Toggle::draw`], shared by this struct's read-only
/// [`Instrument`] impl and by [`InteractiveToggle`], which adds the toggle
/// callback.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly (the
/// `active` flag still drives its lit look); `Clone` so [`crate::ui::PanelSet`]
/// can hand a copy out of its borrowing `get_drawables`.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toggle {
    pub label: String,
    pub active: bool,
}

impl Toggle {
    /// Adds the latching key as its own grown flex node; returns whether it
    /// was clicked this frame. Holds the lit-look override so the inert and
    /// interactive paths render identically. When lit, every pointer state
    /// (rest/hover/press) is forced to the lit look so the key reads as an
    /// engaged lamp, not a momentary button; the style override rides on this
    /// node only.
    fn draw(&self, tui: &mut Tui) -> bool {
        let text = egui::RichText::new(self.label.to_uppercase());
        // Keys grow to share their row: one key per row fills the panel (the
        // selector column look); keys sharing a row split it evenly.
        let node = tui.style(key_style());
        if !self.active {
            return node.ui_add(egui::Button::new(text)).clicked();
        }
        node.mut_egui_style(|style| {
            let widgets = &mut style.visuals.widgets;
            for state in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                state.bg_fill = KEY_LIT;
                state.weak_bg_fill = KEY_LIT;
                state.bg_stroke = Stroke::new(HAIRLINE, ACCENT_GREEN);
                state.fg_stroke = Stroke::new(HAIRLINE, KEY_LIT_TEXT);
            }
        })
        .ui_add(egui::Button::new(text.color(KEY_LIT_TEXT).strong()))
        .clicked()
    }
}

/// The shared key node style: grow to share the row with siblings (a lone key
/// spans the panel; paired keys split it).
pub(super) fn key_style() -> taffy::Style {
    taffy::Style {
        flex_grow: 1.0,
        ..Default::default()
    }
}

impl Instrument for Toggle {
    fn render(&mut self, tui: &mut Tui) {
        // Inert: still clickable, but the click does nothing (e.g. a mock panel).
        let _ = self.draw(tui);
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
    fn render(&mut self, tui: &mut Tui) {
        if self.toggle.draw(tui) {
            (self.on_toggle)();
        }
    }
}
