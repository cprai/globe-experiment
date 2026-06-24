use crate::simulation::{Clock, SimulationState};

/// Left margin (egui points) of the control-panel column.
const UI_MARGIN_X: f32 = 10.0;
/// White, the common readout color.
const UI_WHITE: [u8; 3] = [255, 255, 255];
/// Top Y (egui points) where a scenario should place its per-satellite block,
/// directly below the shared-core block produced by `SimulationState`.
/// Scenarios read this so the two blocks stack without overlapping (absolute
/// positioning - each element owns its screen coordinate).
pub const SCENARIO_UI_TOP_Y: f32 = 140.0;

/// A single self-positioned UI control or readout for one frame, as plain data
/// (no egui types here - egui only enters in [`control_panel`]). Interactivity
/// is carried as *optional* callbacks: a `None` callback renders an inert
/// control, which is what lets the same panel code drive a mock UI.
/// [`control_panel`] maps each element onto egui at its absolute screen
/// `position` (egui points).
///
/// The callbacks are `FnMut` boxed trait objects (an `impl Fn` enum field is
/// not expressible). Their borrow `'a` is the `&mut self` of the producing
/// [`UIDrawable::get_drawables`]; a control's callback captures a *disjoint*
/// field of live state (e.g. one closure mutates `Clock::paused`, another
/// `Clock::multiplier`), so several can coexist without an interior-mutability
/// wrapper.
pub enum UIDrawableElement<'a> {
    /// A static label/readout. `color` is RGB; `strong` bolds it.
    Text {
        position: [f32; 2],
        text: String,
        color: [u8; 3],
        strong: bool,
    },
    /// A clickable button. `on_press` fires on click (when `Some`).
    Button {
        position: [f32; 2],
        label: String,
        on_press: Option<Box<dyn FnMut() + 'a>>,
    },
    /// A value slider over `range`. `on_change` receives the new value on edit
    /// (when `Some`). The producer owns any value mapping (e.g. the speed
    /// slider edits an exponent and its callback exponentiates).
    Slider {
        position: [f32; 2],
        value: f32,
        range: std::ops::RangeInclusive<f32>,
        on_change: Option<Box<dyn FnMut(f32) + 'a>>,
    },
}

/// Anything the control panel can render: it yields a flat list of
/// self-positioned [`UIDrawableElement`]s. Implemented by [`SimulationState`]
/// (the shared-core block) and by each scenario (which prepends the core block,
/// then appends its per-satellite readout). `&mut self` so a control's callback
/// can capture a disjoint mutable field of live state.
pub trait UIDrawable {
    fn get_drawables(&mut self) -> Vec<UIDrawableElement<'_>>;
}

impl UIDrawable for SimulationState {
    /// The shared-core panel block, read from live state: the ephemeris
    /// subsolar point, the clock datetime, and the play/pause + speed
    /// controls whose callbacks mutate the live clock. The two control
    /// callbacks capture disjoint clock fields (`paused` vs `multiplier`)
    /// via direct field assignment - a `Clock` method would borrow the
    /// whole clock and collide.
    fn get_drawables(&mut self) -> Vec<UIDrawableElement<'_>> {
        // Snapshot the displayed values up front (owned `String`/`f32`), so no
        // shared borrow of the clock outlives into the mutable callback captures.
        let subsolar = format!(
            "Sun (subsolar): lat {:.2} deg   lon {:.2} deg",
            self.celestial_sphere.subsolar_lat_deg, self.celestial_sphere.subsolar_lon_deg
        );
        let datetime = format!("Time (UTC): {}", self.clock.datetime_label());
        let speed = format!("Speed: {:.1}x", self.clock.multiplier);
        let play_label = if self.clock.paused { "Play" } else { "Pause" }.to_string();

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock.multiplier.ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        vec![
            UIDrawableElement::Text {
                position: [UI_MARGIN_X, 10.0],
                text: subsolar,
                color: UI_WHITE,
                strong: false,
            },
            UIDrawableElement::Text {
                position: [UI_MARGIN_X, 40.0],
                text: datetime,
                color: UI_WHITE,
                strong: false,
            },
            UIDrawableElement::Button {
                position: [UI_MARGIN_X, 70.0],
                label: play_label,
                on_press: Some(Box::new(|| self.clock.paused = !self.clock.paused)),
            },
            UIDrawableElement::Text {
                position: [90.0, 74.0],
                text: speed,
                color: UI_WHITE,
                strong: false,
            },
            UIDrawableElement::Slider {
                position: [UI_MARGIN_X, 104.0],
                value: speed_exp,
                range: exp_range,
                on_change: Some(Box::new(|exp| self.clock.multiplier = exp.exp())),
            },
        ]
    }
}

/// The control/readout panel over the globe: the simulation clock (play/pause +
/// speed), the ephemeris-driven subsolar point, and each tracked station's
/// datetime and position.
///
/// This function is deliberately *decoupled from interactivity*: it knows
/// nothing about the `Clock` or any scenario. It asks the `drawable` for a flat
/// list of self-positioned [`UIDrawableElement`]s and renders each one at its
/// absolute screen position, treating the simulation-core and scenario blocks
/// identically. Interactivity rides along as optional callbacks on the
/// elements; a control whose callback is `None` renders but does nothing -
/// which is what lets the same code render a mock panel.
pub fn control_panel(ctx: &egui::Context, drawable: &mut impl UIDrawable) {
    // Each element gets its own anchored Area so it can be placed independently.
    for (index, element) in drawable.get_drawables().into_iter().enumerate() {
        let id = egui::Id::new(("ui_drawable", index));
        match element {
            UIDrawableElement::Text {
                position,
                text,
                color,
                strong,
            } => {
                egui::Area::new(id)
                    .fixed_pos(egui::pos2(position[0], position[1]))
                    .show(ctx, |ui| {
                        // An Area sizes itself to the previous frame's content
                        // size, then wraps content to that width - so a widget
                        // that wraps can never grow back, ratcheting smaller
                        // (e.g. on a Play/Pause label toggle). Never wrap.
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        let mut rich = egui::RichText::new(text)
                            .color(egui::Color32::from_rgb(color[0], color[1], color[2]));
                        if strong {
                            rich = rich.strong();
                        }
                        ui.label(rich);
                    });
            }
            UIDrawableElement::Button {
                position,
                label,
                mut on_press,
            } => {
                egui::Area::new(id)
                    .fixed_pos(egui::pos2(position[0], position[1]))
                    .show(ctx, |ui| {
                        // Never wrap: a wrapping label can't grow the Area back
                        // after a shorter label shrank it, so the button would
                        // ratchet smaller on every Play/Pause toggle.
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        if ui.button(label).clicked()
                            && let Some(callback) = on_press.as_mut()
                        {
                            callback();
                        }
                    });
            }
            UIDrawableElement::Slider {
                position,
                value,
                range,
                mut on_change,
            } => {
                egui::Area::new(id)
                    .fixed_pos(egui::pos2(position[0], position[1]))
                    .show(ctx, |ui| {
                        ui.spacing_mut().slider_width = 260.0;
                        let mut edited = value;
                        if ui
                            .add(egui::Slider::new(&mut edited, range).show_value(false))
                            .changed()
                            && let Some(callback) = on_change.as_mut()
                        {
                            callback(edited);
                        }
                    });
            }
        }
    }
}
