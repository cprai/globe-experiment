use egui_taffy::{Tui, TuiBuilderLogic};
use pyo3::prelude::*;

use super::toggle::key_style;
use super::{Callback, Instrument};

/// A momentary key - the inert render data only. Drawing lives in
/// [`Button::draw`], shared by this struct's read-only [`Instrument`] impl and
/// by [`InteractiveButton`], which adds the press callback.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::engine::ui::PanelSet`] can hand a copy out of its
/// borrowing `get_drawables` (and the Python bridge out of its pyclass cell);
/// `pyclass` for the dual Rust/Python UI API.
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
    /// Adds the key as its own grown flex node (see
    /// [`key_style`](super::toggle::key_style)); returns the response so each
    /// wrapper can read its own trigger (click vs held). Holds the widget look
    /// so the inert and interactive paths render identically; the caller picks
    /// the `sense` because the trigger dictates it (see
    /// [`InteractiveHoldButton`]). Wrapping is disabled for the same reason as
    /// the shared `leaf` helper: a longer-than-panel label (e.g. "RETROGRADE")
    /// must widen its panel, not fold onto a second line.
    fn draw(&self, tui: &mut Tui, sense: egui::Sense) -> egui::Response {
        tui.style(key_style())
            .wrap_mode(egui::TextWrapMode::Extend)
            .ui_add(egui::Button::new(self.label.to_uppercase()).sense(sense))
    }
}

impl<S> Instrument<S> for Button {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // Inert: still clickable, but the click does nothing (e.g. a mock panel).
        let _ = self.draw(tui, egui::Sense::click());
    }
}

/// A [`Button`] wired to a press callback. `on_press` fires on click,
/// receiving the live scene (threaded in by `control_panel` at fire time -
/// no capture, so every panel callback coexists). Like every interactive
/// callback it must be idempotent (write-only / snapshot-based): egui's
/// discard pass can fire it twice per frame.
///
/// No live panel drives a click-fired button today (the burn keys hold-fire
/// via [`InteractiveHoldButton`]); this completes the control set as part of
/// the reusable instrument library, so `dead_code` is allowed until a
/// producer constructs one.
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

/// A [`Button`] wired to a hold callback: `on_hold` fires **every frame** the
/// key is held down (e.g. a thruster burn that lasts as long as the press),
/// receiving the live scene like [`InteractiveButton`]. Frames keep coming
/// while held because the burn only matters with the simulation clock
/// running, and a running clock already requests every frame. Same
/// idempotency rule as [`InteractiveButton`] (a per-frame flag set is
/// naturally idempotent).
pub struct InteractiveHoldButton<S> {
    pub button: Button,
    pub on_hold: Callback<S>,
}

impl<S> Instrument<S> for InteractiveHoldButton<S> {
    fn render(&mut self, tui: &mut Tui, scene: &mut S) {
        // Drag sense is load-bearing: egui tracks a held press on a click-only
        // widget as a "potential click" and abandons it after
        // `max_click_duration` (0.8 s) or ~6 px of pointer travel, flipping
        // `is_pointer_button_down_on` false mid-hold. A drag-sensing widget
        // stays the pointer's interaction target until release, so the hold
        // fires for as long as the key is physically down.
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

    /// A minimal drawable: one bottom-center panel holding a single hold key.
    /// `fired` records whether the hold callback ran during the egui pass.
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

    /// Runs one CPU-only egui pass over the probe's panel and reports whether
    /// the hold callback fired.
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
    /// Each probe uses a fresh context; the warmup frame gives egui the widget
    /// rects its hit test reads.
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

    /// A held key must keep firing well past egui's `max_click_duration`
    /// (0.8 s), after which egui abandons the press as a potential *click*;
    /// only the key's drag sense keeps the interaction (and thus the burn)
    /// alive. Regression test for the burn keys cutting out mid-hold.
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

        // Hold with no further pointer events: only time advances, exactly
        // like a user pinning the key down. 3 s is well past the 0.8 s click
        // timeout that used to cut the burn short.
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
