use egui_taffy::{Tui, TuiBuilderLogic};
use pyo3::prelude::*;

use super::toggle::key_style;
use super::{Callback, Instrument};

/// A momentary key - the inert render data only; [`Button::draw`] is shared
/// with the interactive wrappers.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(module = "globe", from_py_object)]
pub struct Button {
    #[pyo3(get, set)]
    pub label: String,
}

#[pymethods]
impl Button {
    #[new]
    fn py_new(label: String) -> Self {
        Self { label }
    }
}

impl Button {
    /// Adds the key as its own grown flex node; returns the response so each
    /// wrapper reads its own trigger, and the caller picks `sense` because
    /// the trigger dictates it (see [`InteractiveHoldButton`]). Wrapping is
    /// disabled: a long label (e.g. "RETROGRADE") must widen its panel, not
    /// fold onto a second line.
    fn draw(&self, tui: &mut Tui, sense: egui::Sense) -> egui::Response {
        tui.style(key_style())
            .wrap_mode(egui::TextWrapMode::Extend)
            .ui_add(egui::Button::new(self.label.to_uppercase()).sense(sense))
    }
}

impl<S> Instrument<S> for Button {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // Inert: still clickable, but the click does nothing (mock panels).
        let _ = self.draw(tui, egui::Sense::click());
    }
}

/// A [`Button`] wired to an `on_press` click callback (idempotent, like every
/// interactive callback - the discard pass can fire it twice per frame).
///
/// No live producer constructs one yet; it completes the reusable instrument
/// library, so `dead_code` is allowed.
#[allow(dead_code)]
pub struct InteractiveButton<S> {
    pub button: Button,
    pub on_press: Callback<S>,
}

impl<S> Instrument<S> for InteractiveButton<S> {
    fn render(&mut self, tui: &mut Tui, scene: &mut S) {
        if self.button.draw(tui, egui::Sense::click()).clicked() {
            (self.on_press)(scene);
        }
    }
}

/// A [`Button`] whose `on_hold` fires **every frame** the key is held (e.g. a
/// thruster burn lasting as long as the press). A per-frame flag set is
/// naturally idempotent under the discard pass.
pub struct InteractiveHoldButton<S> {
    pub button: Button,
    pub on_hold: Callback<S>,
}

impl<S> Instrument<S> for InteractiveHoldButton<S> {
    fn render(&mut self, tui: &mut Tui, scene: &mut S) {
        // Drag sense is load-bearing: egui abandons a held press on a
        // click-only widget after `max_click_duration` (0.8 s) or ~6 px of
        // travel, flipping `is_pointer_button_down_on` false mid-hold. A
        // drag-sensing widget stays the interaction target until release.
        if self
            .button
            .draw(tui, egui::Sense::click_and_drag())
            .is_pointer_button_down_on()
        {
            (self.on_hold)(scene);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ui::{self, PanelAnchor, UIDrawable, UIDrawablePanel};

    /// One bottom-center panel holding a single hold key; `fired` records
    /// whether the hold callback ran during the egui pass.
    struct HoldProbe {
        fired: bool,
    }

    impl UIDrawable for HoldProbe {
        fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
            vec![UIDrawablePanel {
                anchor: PanelAnchor::BottomCenter,
                rows: vec![vec![Box::new(InteractiveHoldButton {
                    button: Button {
                        label: "Hold".to_string(),
                    },
                    on_hold: Box::new(|probe: &mut HoldProbe| probe.fired = true),
                })]],
            }]
        }
    }

    const SCREEN: egui::Vec2 = egui::vec2(400.0, 400.0);

    /// Runs one CPU-only egui pass and reports whether the hold callback
    /// fired.
    fn run_frame(
        ctx: &egui::Context,
        probe: &mut HoldProbe,
        time: f64,
        events: Vec<egui::Event>,
    ) -> bool {
        probe.fired = false;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
            time: Some(time),
            events,
            ..Default::default()
        };
        // Panels built once before run_ui, like the live redraw path.
        let mut panels = probe.get_drawables();
        let _ = ctx.run_ui(input, |ui| ui::control_panel(ui.ctx(), &mut panels, probe));
        probe.fired
    }

    fn press_events(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ]
    }

    /// Finds a point inside the key by probing presses up the bottom-center
    /// panel (taffy sizes it to content, so the rect is not known a priori).
    /// The warmup frame gives egui the widget rects its hit test reads.
    fn find_point_on_key() -> egui::Pos2 {
        let x = SCREEN.x / 2.0;
        let mut y = SCREEN.y - 4.0;
        while y > 0.0 {
            let pos = egui::pos2(x, y);
            let ctx = egui::Context::default();
            ui::install_theme(&ctx);
            let mut probe = HoldProbe { fired: false };
            run_frame(&ctx, &mut probe, 0.0, Vec::new());
            if run_frame(&ctx, &mut probe, 0.05, press_events(pos)) {
                return pos;
            }
            y -= 2.0;
        }
        panic!("no press position reached the hold key");
    }

    /// A held key must keep firing well past egui's 0.8 s
    /// `max_click_duration`; only the drag sense keeps the interaction alive.
    /// Regression test for the burn keys cutting out mid-hold.
    #[test]
    fn hold_key_fires_past_click_timeout() {
        let pos = find_point_on_key();

        let ctx = egui::Context::default();
        ui::install_theme(&ctx);
        let mut probe = HoldProbe { fired: false };
        run_frame(&ctx, &mut probe, 0.0, Vec::new());
        assert!(
            run_frame(&ctx, &mut probe, 0.05, press_events(pos)),
            "hold key did not fire on the initial press"
        );

        // Hold with no further pointer events - only time advances, like a
        // user pinning the key down. 3 s is well past the 0.8 s timeout.
        let mut time = 0.05;
        while time < 3.0 {
            time += 0.1;
            assert!(
                run_frame(&ctx, &mut probe, time, Vec::new()),
                "hold key stopped firing at t = {time:.2} s while still held"
            );
        }
    }
}
