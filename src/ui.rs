use egui::{Color32, CornerRadius, Margin, Shadow, Stroke};

use crate::simulation::{Clock, SimulationState};

// ---------------------------------------------------------------------------
// Apollo-panel theme palette.
//
// The look: a rugged, dark gunmetal instrument panel floating over the bright
// globe, with cream "lit readout" text and keys that light green when engaged.
// References: real Apollo CSM/LM panels (gunmetal + engraved labels + lamp
// accents) and the game UI in `ui_examples/`. Every color is theme-internal:
// producers (scenarios / SimulationState) pick *which instrument* to draw, not
// its color - the style lives entirely here and in `render_element`.
// ---------------------------------------------------------------------------

/// Cream "lit readout" color for instrument *values* - warm off-white reads as
/// a backlit digit window, not a flat white sticker.
const READOUT_CREAM: Color32 = Color32::from_rgb(222, 214, 184);
/// Dim engraved tone for instrument *labels* (the caption beside a value), so a
/// label recedes and its value reads as the lit element.
const LABEL_DIM: Color32 = Color32::from_rgb(150, 156, 150);
/// Amber accent for a section header (the title atop a cluster).
const HEADER_AMBER: Color32 = Color32::from_rgb(230, 178, 86);
/// Red fault-lamp tone.
const ACCENT_RED: Color32 = Color32::from_rgb(214, 92, 76);

/// Panel body: near-black blue-gray gunmetal, slightly translucent so the globe
/// shows faintly through the instrument cluster.
const PANEL_FILL: Color32 = Color32::from_rgba_unmultiplied_const(24, 28, 32, 236);
/// Recessed/inset field (egui's extreme/faint backgrounds, e.g. the slider
/// track): darker than the panel, like a cut-in readout well.
const RECESS_FILL: Color32 = Color32::from_rgb(12, 14, 16);
/// Raised-edge highlight painted along a panel's top/left (the lit metal lip).
const BEVEL_LIGHT: Color32 = Color32::from_rgb(92, 102, 110);
/// The panel outline / dark recess edge (bottom-right of the faked bevel).
const BEVEL_DARK: Color32 = Color32::from_rgb(8, 10, 12);

/// A key (button) at rest: brushed gunmetal, lighter than the panel body.
const KEY_FILL: Color32 = Color32::from_rgb(46, 52, 57);
/// Key outline at rest.
const KEY_EDGE: Color32 = Color32::from_rgb(80, 90, 98);
/// Key under the pointer.
const KEY_HOVER: Color32 = Color32::from_rgb(58, 66, 72);
/// Key while pressed/engaged: green-tinted, echoing the lit lamp accents.
const KEY_ACTIVE: Color32 = Color32::from_rgb(40, 66, 46);
/// Lamp-green accent for engaged keys and the slider grab/trail.
const ACCENT_GREEN: Color32 = Color32::from_rgb(122, 214, 130);
/// Screw-head metal for the corner rivets.
const RIVET_BODY: Color32 = Color32::from_rgb(96, 104, 110);
/// Screw-slot / shadow for the corner rivets.
const RIVET_SLOT: Color32 = Color32::from_rgb(20, 24, 28);

/// Which screen corner a [`UIDrawablePanel`] anchors to. egui-free (mapped to
/// an `egui::Align2` in [`control_panel`]); anchoring keeps a panel pinned to
/// its corner as the window resizes. Only the corners currently in use are
/// listed - add the bottom corners when a panel needs one.
///
/// `Copy` so a [`MockUi`] can hand it out of a borrowing `get_drawables` by
/// value; `Deserialize` so the `render --scene` `ui` JSON can name a corner
/// (`"top_left"` / `"top_right"`).
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// State of a [`UIDrawableElement::Lamp`] - the *semantic* condition a producer
/// selects, mapped to a lamp color in [`render_element`] (the producer never
/// names a color). Serde snake_case so the mock JSON can say `"status": "ok"`.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LampStatus {
    /// Nominal - green.
    Ok,
    /// Caution - amber.
    Caution,
    /// Fault - red.
    Fault,
    /// Unlit/inactive - dim.
    Off,
}

/// One pre-styled *instrument* for a frame, as plain data (no egui types here -
/// egui only enters in [`control_panel`]). Each variant is a specific display
/// with a baked-in look; a producer picks *which* instrument and supplies its
/// content (and, for controls, a callback), but **never** its color, font, or
/// emphasis - that all lives in [`render_element`]. [`control_panel`] places
/// each instrument at its `position`, **relative to its containing
/// [`UIDrawablePanel`]'s** content origin (egui points).
///
/// Control callbacks are `FnMut` boxed trait objects (an `impl Fn` enum field
/// is not expressible) and *optional*: a `None` callback renders an inert
/// control, which is what lets the same code drive a mock UI. Their borrow `'a`
/// is the `&mut self` of the producing [`UIDrawable::get_drawables`]; each
/// captures a *disjoint* field of live state (e.g. one closure mutates
/// `Clock::paused`, another `Clock::multiplier`), so several coexist without
/// interior mutability.
pub enum UIDrawableElement<'a> {
    /// A section header: a bold amber title with a rule ruled beneath it.
    Header { position: [f32; 2], title: String },
    /// A labelled value: a dim caption beside a cream value in a recessed
    /// readout window.
    Readout {
        position: [f32; 2],
        label: String,
        value: String,
    },
    /// Two labelled values side by side on one row, for compact paired readouts
    /// (e.g. LAT / LON).
    DualReadout {
        position: [f32; 2],
        left_label: String,
        left_value: String,
        right_label: String,
        right_value: String,
    },
    /// A momentary key. `on_press` fires on click (when `Some`).
    Button {
        position: [f32; 2],
        label: String,
        on_press: Option<Box<dyn FnMut() + 'a>>,
    },
    /// A latching key that lights green while `active`. `on_toggle` fires on
    /// click (when `Some`); the producer owns flipping the state it reflects.
    Toggle {
        position: [f32; 2],
        label: String,
        active: bool,
        on_toggle: Option<Box<dyn FnMut() + 'a>>,
    },
    /// A status indicator lamp (colored dot keyed to `status`) plus a caption.
    Lamp {
        position: [f32; 2],
        label: String,
        status: LampStatus,
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

/// Owned, callback-free mirror of one [`UIDrawableElement`], deserialized from
/// the `render --scene` `ui` JSON. The live control variants carry `Box<dyn
/// FnMut>` callbacks (not serializable); a mock panel is inert, so these carry
/// only the rendered data (a `Toggle`'s `active` still drives its lit look).
/// Tagged by instrument name in snake_case, e.g.
/// `{"readout": {"position": [0, 0], "label": "ALT", "value": "417 km"}}`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum UiElementSpec {
    /// A section header.
    Header { position: [f32; 2], title: String },
    /// A labelled value readout.
    Readout {
        position: [f32; 2],
        label: String,
        value: String,
    },
    /// Two labelled values on one row.
    DualReadout {
        position: [f32; 2],
        left_label: String,
        left_value: String,
        right_label: String,
        right_value: String,
    },
    /// An inert momentary key (renders, does nothing).
    Button { position: [f32; 2], label: String },
    /// An inert latching key; `active` drives the lit look.
    Toggle {
        position: [f32; 2],
        label: String,
        active: bool,
    },
    /// A status indicator lamp.
    Lamp {
        position: [f32; 2],
        label: String,
        status: LampStatus,
    },
    /// An inert slider. `range` is `[min, max]`.
    Slider {
        position: [f32; 2],
        value: f32,
        range: [f32; 2],
    },
}

/// Owned, callback-free mirror of one [`UIDrawablePanel`], deserialized from
/// the `render --scene` `ui` JSON: a corner `anchor`, an inset `offset`, a
/// fixed box `size`, and panel-relative `elements`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPanelSpec {
    pub anchor: PanelAnchor,
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub elements: Vec<UiElementSpec>,
}

/// A set of mock panels (from the `render --scene` `ui` section) that renders
/// through the exact same [`control_panel`] path as the live UI, so a mock
/// layout is faithful to real output. Every control is inert: its callback is
/// `None`.
pub struct MockUi {
    pub panels: Vec<UiPanelSpec>,
}

impl UIDrawable for MockUi {
    /// Maps each owned [`UiPanelSpec`] to a borrowed [`UIDrawablePanel`] with
    /// every callback `None` - the same shape a live `get_drawables` returns,
    /// minus interactivity.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        self.panels
            .iter()
            .map(|panel| UIDrawablePanel {
                anchor: panel.anchor,
                offset: panel.offset,
                size: panel.size,
                elements: panel
                    .elements
                    .iter()
                    .map(|element| match element {
                        UiElementSpec::Header { position, title } => UIDrawableElement::Header {
                            position: *position,
                            title: title.clone(),
                        },
                        UiElementSpec::Readout {
                            position,
                            label,
                            value,
                        } => UIDrawableElement::Readout {
                            position: *position,
                            label: label.clone(),
                            value: value.clone(),
                        },
                        UiElementSpec::DualReadout {
                            position,
                            left_label,
                            left_value,
                            right_label,
                            right_value,
                        } => UIDrawableElement::DualReadout {
                            position: *position,
                            left_label: left_label.clone(),
                            left_value: left_value.clone(),
                            right_label: right_label.clone(),
                            right_value: right_value.clone(),
                        },
                        UiElementSpec::Button { position, label } => UIDrawableElement::Button {
                            position: *position,
                            label: label.clone(),
                            on_press: None,
                        },
                        UiElementSpec::Toggle {
                            position,
                            label,
                            active,
                        } => UIDrawableElement::Toggle {
                            position: *position,
                            label: label.clone(),
                            active: *active,
                            on_toggle: None,
                        },
                        UiElementSpec::Lamp {
                            position,
                            label,
                            status,
                        } => UIDrawableElement::Lamp {
                            position: *position,
                            label: label.clone(),
                            status: *status,
                        },
                        UiElementSpec::Slider {
                            position,
                            value,
                            range,
                        } => UIDrawableElement::Slider {
                            position: *position,
                            value: *value,
                            range: range[0]..=range[1],
                            on_change: None,
                        },
                    })
                    .collect(),
            })
            .collect()
    }
}

impl UIDrawable for SimulationState {
    /// The shared-core panel, read from live state: the ephemeris subsolar
    /// point, the clock datetime, and the play/pause + speed controls whose
    /// callbacks mutate the live clock. The two control callbacks capture
    /// disjoint clock fields (`paused` vs `multiplier`) via direct field
    /// assignment - a `Clock` method would borrow the whole clock and collide.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // Snapshot the displayed values up front (owned `String`/`f32`/`bool`),
        // so no shared borrow of the clock outlives into the mutable callback
        // captures below.
        let datetime = self.clock.datetime_label();
        let subsolar_lat = format!("{:.2} deg", self.celestial_sphere.subsolar_lat_deg);
        let subsolar_lon = format!("{:.2} deg", self.celestial_sphere.subsolar_lon_deg);
        let speed = format!("{:.1}x", self.clock.multiplier);
        let running = !self.clock.paused;

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock.multiplier.ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        // Instrument positions are relative to this panel's content origin. The
        // producer picks instruments + content only; all styling is in the ui
        // module.
        let elements = vec![
            UIDrawableElement::Header {
                position: [0.0, 0.0],
                title: "Time / Subsolar".to_string(),
            },
            UIDrawableElement::Readout {
                position: [0.0, 26.0],
                label: "UTC".to_string(),
                value: datetime,
            },
            UIDrawableElement::DualReadout {
                position: [0.0, 52.0],
                left_label: "Lat".to_string(),
                left_value: subsolar_lat,
                right_label: "Lon".to_string(),
                right_value: subsolar_lon,
            },
            UIDrawableElement::Toggle {
                position: [0.0, 84.0],
                label: "Run".to_string(),
                active: running,
                on_toggle: Some(Box::new(|| self.clock.paused = !self.clock.paused)),
            },
            UIDrawableElement::Readout {
                position: [104.0, 86.0],
                label: "Speed".to_string(),
                value: speed,
            },
            UIDrawableElement::Slider {
                position: [0.0, 114.0],
                value: speed_exp,
                range: exp_range,
                on_change: Some(Box::new(|exp| self.clock.multiplier = exp.exp())),
            },
        ];

        vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            offset: [10.0, 10.0],
            size: [340.0, 148.0],
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

/// Installs the Apollo-panel theme onto an egui [`Context`](egui::Context):
/// monospace everywhere, the gunmetal palette, and beveled keys that light
/// green when engaged. Call once per context, right after creating it - both
/// the windowed app and the headless render path do so, so the live UI and a
/// mock overlay share one look.
pub fn install_theme(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};

    let mut style = (*ctx.global_style()).clone();

    // Monospace everywhere - the instrument-readout feel. egui ships the "Hack"
    // monospace family, so this needs no font asset.
    let mono = FontFamily::Monospace;
    style.text_styles = [
        (TextStyle::Heading, FontId::new(15.0, mono.clone())),
        (TextStyle::Body, FontId::new(13.0, mono.clone())),
        (TextStyle::Monospace, FontId::new(13.0, mono.clone())),
        (TextStyle::Button, FontId::new(13.0, mono.clone())),
        (TextStyle::Small, FontId::new(11.0, mono)),
    ]
    .into();

    let key_radius = CornerRadius::same(3);
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = PANEL_FILL;
    v.window_fill = PANEL_FILL;
    // Inset wells (slider track, etc.) read as cut-in readout fields.
    v.extreme_bg_color = RECESS_FILL;
    v.faint_bg_color = RECESS_FILL;
    v.slider_trailing_fill = true;
    v.selection.bg_fill = ACCENT_GREEN.gamma_multiply(0.5);
    v.selection.stroke = Stroke::new(1.0, ACCENT_GREEN);

    // Widget states: brushed-gunmetal keys that light green on hover/press.
    let w = &mut v.widgets;
    let readout = Stroke::new(1.0, READOUT_CREAM);

    w.noninteractive.bg_fill = PANEL_FILL;
    w.noninteractive.weak_bg_fill = PANEL_FILL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BEVEL_DARK);
    w.noninteractive.fg_stroke = readout;
    w.noninteractive.corner_radius = key_radius;

    w.inactive.bg_fill = KEY_FILL;
    w.inactive.weak_bg_fill = KEY_FILL;
    w.inactive.bg_stroke = Stroke::new(1.0, KEY_EDGE);
    w.inactive.fg_stroke = readout;
    w.inactive.corner_radius = key_radius;

    w.hovered.bg_fill = KEY_HOVER;
    w.hovered.weak_bg_fill = KEY_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT_GREEN.gamma_multiply(0.8));
    w.hovered.fg_stroke = Stroke::new(1.0, ACCENT_GREEN);
    w.hovered.corner_radius = key_radius;
    w.hovered.expansion = 1.0;

    w.active.bg_fill = KEY_ACTIVE;
    w.active.weak_bg_fill = KEY_ACTIVE;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT_GREEN);
    w.active.fg_stroke = Stroke::new(1.0, ACCENT_GREEN);
    w.active.corner_radius = key_radius;
    w.active.expansion = 1.0;

    w.open = w.active;

    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);

    ctx.set_global_style(style);
}

/// The gunmetal panel frame: dark fill, a dark outline (the raised lip's
/// highlight is painted separately in [`control_panel`]), small radius, a drop
/// shadow to lift it off the globe, and a generous inner margin so the contents
/// sit inboard of the rivet line.
fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL_FILL)
        .stroke(Stroke::new(1.0, BEVEL_DARK))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::same(12))
        .shadow(Shadow {
            offset: [2, 3],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(130),
        })
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
                let framed = panel_frame().show(ui, |ui| {
                    // Fix the box to the requested size: it makes the frame a
                    // consistent rectangle and pins the Area size so it can't
                    // auto-shrink against the previous frame's content.
                    ui.set_min_size(size);
                    let origin = ui.min_rect().min;
                    for element in panel.elements {
                        render_element(ui, origin, size, element);
                    }
                });
                // Fake an extruded-metal bevel: a single light edge along the
                // top and left, inboard of the dark frame outline. The dark
                // bottom/right is the frame stroke itself, so the panel reads as
                // a raised key cluster, not a flat card.
                paint_bevel(ui.painter(), framed.response.rect);
                // Slotted screws in the corners - the bolted-down look of the
                // real Apollo panels.
                paint_rivets(ui.painter(), framed.response.rect);
            });
    }
}

/// Paints the raised-lip highlight (top + left edges) just inside a panel's
/// outline. See [`control_panel`].
fn paint_bevel(painter: &egui::Painter, rect: egui::Rect) {
    let inner = rect.shrink(1.0);
    let stroke = Stroke::new(1.0, BEVEL_LIGHT);
    painter.line_segment([inner.left_top(), inner.right_top()], stroke);
    painter.line_segment([inner.left_top(), inner.left_bottom()], stroke);
}

/// Paints a slotted screw head inset from each corner of a panel - the
/// bolted-down hardware look from the Apollo references. Each is a small metal
/// disc with a dark outline and a horizontal slot.
fn paint_rivets(painter: &egui::Painter, rect: egui::Rect) {
    let inset = egui::vec2(8.0, 8.0);
    let radius = 2.8;
    let corners = [
        rect.left_top() + egui::vec2(inset.x, inset.y),
        rect.right_top() + egui::vec2(-inset.x, inset.y),
        rect.left_bottom() + egui::vec2(inset.x, -inset.y),
        rect.right_bottom() + egui::vec2(-inset.x, -inset.y),
    ];
    let slot = Stroke::new(1.0, RIVET_SLOT);
    for center in corners {
        painter.circle(center, radius, RIVET_BODY, Stroke::new(1.0, BEVEL_DARK));
        painter.line_segment(
            [
                center + egui::vec2(-radius * 0.6, 0.0),
                center + egui::vec2(radius * 0.6, 0.0),
            ],
            slot,
        );
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
            UIDrawableElement::Header { title, .. } => {
                // Larger, bold amber title with a rule ruled across the panel
                // width - the labelled divider that tops each cluster on the
                // Apollo panels.
                let rule_y = child_rect.top() + 19.0;
                ui.painter().hline(
                    child_rect.left()..=child_rect.left() + size.x,
                    rule_y,
                    Stroke::new(1.0, BEVEL_LIGHT),
                );
                ui.label(
                    egui::RichText::new(title.to_uppercase())
                        .color(HEADER_AMBER)
                        .strong()
                        .size(15.0),
                );
            }
            UIDrawableElement::Readout { label, value, .. } => {
                ui.horizontal(|ui| readout_pair(ui, &label, &value));
            }
            UIDrawableElement::DualReadout {
                left_label,
                left_value,
                right_label,
                right_value,
                ..
            } => {
                ui.horizontal(|ui| {
                    readout_pair(ui, &left_label, &left_value);
                    ui.add_space(14.0);
                    readout_pair(ui, &right_label, &right_value);
                });
            }
            UIDrawableElement::Button {
                label,
                mut on_press,
                ..
            } => {
                if ui.button(label.to_uppercase()).clicked()
                    && let Some(callback) = on_press.as_mut()
                {
                    callback();
                }
            }
            UIDrawableElement::Toggle {
                label,
                active,
                mut on_toggle,
                ..
            } => {
                if toggle_key(ui, &label, active).clicked()
                    && let Some(callback) = on_toggle.as_mut()
                {
                    callback();
                }
            }
            UIDrawableElement::Lamp { label, status, .. } => {
                render_lamp(ui, &label, status);
            }
            UIDrawableElement::Slider {
                value,
                range,
                mut on_change,
                ..
            } => {
                ui.spacing_mut().slider_width = 280.0;
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

/// Renders a dim engraved label beside its value, the value sitting in a
/// recessed cream-on-black readout window - the label/value split that keeps a
/// value reading as the lit element. Called inside a horizontal layout.
fn readout_pair(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label.to_uppercase()).color(LABEL_DIM));
    egui::Frame::new()
        .fill(RECESS_FILL)
        .stroke(Stroke::new(1.0, BEVEL_DARK))
        .corner_radius(CornerRadius::same(2))
        .inner_margin(Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(value.to_uppercase()).color(READOUT_CREAM));
        });
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

/// Renders a status indicator lamp (a colored disc in a recessed socket) plus
/// its dim caption. The `status` picks the lamp color here - the producer only
/// names the condition.
fn render_lamp(ui: &mut egui::Ui, label: &str, status: LampStatus) {
    let color = match status {
        LampStatus::Ok => ACCENT_GREEN,
        LampStatus::Caution => HEADER_AMBER,
        LampStatus::Fault => ACCENT_RED,
        LampStatus::Off => LABEL_DIM,
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        let center = rect.center();
        let painter = ui.painter();
        painter.circle_filled(center, 6.0, RECESS_FILL);
        // A soft halo, then the lit disc.
        painter.circle_filled(center, 5.0, color.gamma_multiply(0.35));
        painter.circle_filled(center, 3.2, color);
        painter.circle_stroke(center, 6.0, Stroke::new(1.0, BEVEL_DARK));
        ui.label(egui::RichText::new(label.to_uppercase()).color(LABEL_DIM));
    });
}

/// The panel-relative position carried by any instrument variant.
fn element_position(element: &UIDrawableElement<'_>) -> [f32; 2] {
    match element {
        UIDrawableElement::Header { position, .. }
        | UIDrawableElement::Readout { position, .. }
        | UIDrawableElement::DualReadout { position, .. }
        | UIDrawableElement::Button { position, .. }
        | UIDrawableElement::Toggle { position, .. }
        | UIDrawableElement::Lamp { position, .. }
        | UIDrawableElement::Slider { position, .. } => *position,
    }
}
