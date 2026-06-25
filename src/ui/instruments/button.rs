use super::Instrument;

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
    pub position: [f32; 2],
    pub label: String,
}

impl Button {
    /// Draws the key into `ui`; returns whether it was clicked this frame.
    /// Holds the widget look so the inert and interactive paths render
    /// identically.
    fn draw(&self, ui: &mut egui::Ui) -> bool {
        ui.button(self.label.to_uppercase()).clicked()
    }
}

impl Instrument for Button {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        // Inert: still clickable, but the click does nothing (e.g. a mock panel).
        let _ = self.draw(ui);
    }
}

/// A [`Button`] wired to a press callback. `on_press` fires on click. The
/// borrow `'a` is the `&mut self` of the producing
/// [`crate::ui::UIDrawable::get_drawables`], so the closure can capture a
/// disjoint mutable field of live state.
///
/// No live panel drives a momentary button today; this completes the control
/// set as part of the reusable instrument library, so `dead_code` is allowed
/// until a producer constructs one.
#[allow(dead_code)]
pub struct InteractiveButton<'a> {
    pub button: Button,
    pub on_press: Box<dyn FnMut() + 'a>,
}

impl Instrument for InteractiveButton<'_> {
    fn position(&self) -> [f32; 2] {
        self.button.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        if self.button.draw(ui) {
            (self.on_press)();
        }
    }
}
