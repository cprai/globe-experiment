use super::Instrument;

/// A momentary key. `on_press` fires on click (when `Some`); a `None` callback
/// renders an inert key (e.g. for a mock panel).
pub struct Button<'a> {
    pub position: [f32; 2],
    pub label: String,
    pub on_press: Option<Box<dyn FnMut() + 'a>>,
}

impl Instrument for Button<'_> {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        if ui.button(self.label.to_uppercase()).clicked()
            && let Some(callback) = self.on_press.as_mut()
        {
            callback();
        }
    }
}
