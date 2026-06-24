use crate::simulation::{Clock, SimulationState};

/// White, the common readout color.
const UI_WHITE: [u8; 3] = [255, 255, 255];

/// Which screen corner a [`UIDrawablePanel`] anchors to. egui-free (mapped to
/// an `egui::Align2` in [`control_panel`]); anchoring keeps a panel pinned to
/// its corner as the window resizes. Only the corners currently in use are
/// listed - add the bottom corners when a panel needs one.
pub enum PanelAnchor {
    TopLeft,
    TopRight,
}

/// One positioned group of UI elements for a frame. The panel owns its place on
/// screen (a corner `anchor` plus an inset `offset`, both resolved against the
/// live window in [`control_panel`]); its `elements` carry positions *relative*
/// to the panel's content origin. `size` fixes the panel's box - it both sizes
/// the frame and pins the egui `Area` size so it can't auto-shrink frame to
/// frame.
pub struct UIDrawablePanel<'a> {
    pub anchor: PanelAnchor,
    /// Inset (egui points) from the anchored corner, toward the screen
    /// interior.
    pub offset: [f32; 2],
    /// Panel box size (egui points).
    pub size: [f32; 2],
    pub elements: Vec<UIDrawableElement<'a>>,
}

/// A single UI control or readout for one frame, as plain data (no egui types
/// here - egui only enters in [`control_panel`]). Interactivity is carried as
/// *optional* callbacks: a `None` callback renders an inert control, which is
/// what lets the same panel code drive a mock UI. [`control_panel`] places each
/// element at its `position`, **relative to its containing
/// [`UIDrawablePanel`]'s** content origin (egui points).
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

/// Anything the control panel can render: it yields a list of positioned
/// [`UIDrawablePanel`]s, each owning a group of relatively-placed
/// [`UIDrawableElement`]s. Implemented by [`SimulationState`] (one shared-core
/// panel) and by each scenario (which returns the core panel plus its own
/// per-satellite panel). `&mut self` so a control's callback can capture a
/// disjoint mutable field of live state.
pub trait UIDrawable {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>>;
}

impl UIDrawable for SimulationState {
    /// The shared-core panel, read from live state: the ephemeris subsolar
    /// point, the clock datetime, and the play/pause + speed controls whose
    /// callbacks mutate the live clock. The two control callbacks capture
    /// disjoint clock fields (`paused` vs `multiplier`) via direct field
    /// assignment - a `Clock` method would borrow the whole clock and collide.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
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

        // Element positions are relative to this panel's content origin.
        let elements = vec![
            UIDrawableElement::Text {
                position: [0.0, 0.0],
                text: subsolar,
                color: UI_WHITE,
                strong: false,
            },
            UIDrawableElement::Text {
                position: [0.0, 30.0],
                text: datetime,
                color: UI_WHITE,
                strong: false,
            },
            UIDrawableElement::Button {
                position: [0.0, 60.0],
                label: play_label,
                on_press: Some(Box::new(|| self.clock.paused = !self.clock.paused)),
            },
            UIDrawableElement::Text {
                position: [80.0, 64.0],
                text: speed,
                color: UI_WHITE,
                strong: false,
            },
            UIDrawableElement::Slider {
                position: [0.0, 94.0],
                value: speed_exp,
                range: exp_range,
                on_change: Some(Box::new(|exp| self.clock.multiplier = exp.exp())),
            },
        ];

        vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            offset: [10.0, 10.0],
            size: [300.0, 130.0],
            elements,
        }]
    }
}

/// Maps a [`PanelAnchor`] + inset to egui's `Area::anchor` arguments. egui's
/// offset is measured from the anchored corner toward the screen interior, so
/// the inset is negated on the right/bottom edges.
fn anchor_to_egui(anchor: &PanelAnchor, offset: [f32; 2]) -> (egui::Align2, egui::Vec2) {
    let [x, y] = offset;
    match anchor {
        PanelAnchor::TopLeft => (egui::Align2::LEFT_TOP, egui::vec2(x, y)),
        PanelAnchor::TopRight => (egui::Align2::RIGHT_TOP, egui::vec2(-x, y)),
    }
}

/// The control/readout panel(s) over the globe: one panel holds the simulation
/// clock (play/pause and speed) plus the ephemeris-driven subsolar point;
/// another holds each tracked station's datetime and position.
///
/// This function is deliberately *decoupled from interactivity*: it knows
/// nothing about the `Clock` or any scenario. It asks the `drawable` for a list
/// of [`UIDrawablePanel`]s, frames each at its anchored position, and renders
/// each panel's [`UIDrawableElement`]s at their panel-relative positions.
/// Interactivity rides along as optional callbacks; a control whose callback is
/// `None` renders but does nothing - which is what lets the same code render a
/// mock panel.
pub fn control_panel(ctx: &egui::Context, drawable: &mut impl UIDrawable) {
    for (panel_index, panel) in drawable.get_drawables().into_iter().enumerate() {
        let (align, offset) = anchor_to_egui(&panel.anchor, panel.offset);
        let size = egui::vec2(panel.size[0], panel.size[1]);
        egui::Area::new(egui::Id::new(("ui_panel", panel_index)))
            .anchor(align, offset)
            .show(ctx, |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    // Fix the box to the requested size: it makes the frame a
                    // consistent rectangle and pins the Area size so it can't
                    // auto-shrink against the previous frame's content.
                    ui.set_min_size(size);
                    let origin = ui.min_rect().min;
                    for element in panel.elements {
                        render_element(ui, origin, size, element);
                    }
                });
            });
    }
}

/// Renders one element inside a panel at its panel-relative position. Each
/// element gets its own child `Ui` anchored at `origin + position`, extending
/// to the panel's bottom-right so the widget lays out top-left from there.
/// Wrapping is disabled everywhere: an auto-wrapping label can't grow its area
/// back after a shorter label shrank it, so a Play/Pause toggle would ratchet
/// smaller.
fn render_element(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    size: egui::Vec2,
    element: UIDrawableElement<'_>,
) {
    let position = element_position(&element);
    let child_rect =
        egui::Rect::from_min_max(origin + egui::vec2(position[0], position[1]), origin + size);
    let builder = egui::UiBuilder::new()
        .max_rect(child_rect)
        .layout(egui::Layout::top_down(egui::Align::Min));
    ui.scope_builder(builder, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        match element {
            UIDrawableElement::Text {
                text,
                color,
                strong,
                ..
            } => {
                let mut rich = egui::RichText::new(text)
                    .color(egui::Color32::from_rgb(color[0], color[1], color[2]));
                if strong {
                    rich = rich.strong();
                }
                ui.label(rich);
            }
            UIDrawableElement::Button {
                label,
                mut on_press,
                ..
            } => {
                if ui.button(label).clicked()
                    && let Some(callback) = on_press.as_mut()
                {
                    callback();
                }
            }
            UIDrawableElement::Slider {
                value,
                range,
                mut on_change,
                ..
            } => {
                ui.spacing_mut().slider_width = 260.0;
                let mut edited = value;
                if ui
                    .add(egui::Slider::new(&mut edited, range).show_value(false))
                    .changed()
                    && let Some(callback) = on_change.as_mut()
                {
                    callback(edited);
                }
            }
        }
    });
}

/// The panel-relative position carried by any element variant.
fn element_position(element: &UIDrawableElement<'_>) -> [f32; 2] {
    match element {
        UIDrawableElement::Text { position, .. }
        | UIDrawableElement::Button { position, .. }
        | UIDrawableElement::Slider { position, .. } => *position,
    }
}
