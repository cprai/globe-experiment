//! Apollo-panel theme: the palette, the egui style install, and the panel
//! chrome (frame, bevel, rivets).
//!
//! The look: a rugged, dark gunmetal instrument panel floating over the bright
//! globe, with cream "lit readout" text and keys that light green when engaged.
//! References: real Apollo CSM/LM panels (gunmetal + engraved labels + lamp
//! accents) and the game UI in `ui_examples/`. Every color is theme-internal:
//! producers (scenarios / `SimulationState`) pick *which instrument* to draw,
//! not its color - the style lives here and is consumed by the instrument
//! renderers (each owns its own `Instrument::render`).

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke};

// ---------------------------------------------------------------------------
// Apollo-panel palette. Shared with the instrument modules (each instrument's
// render pulls the colors it needs from here), so these are crate-visible.
// ---------------------------------------------------------------------------

/// Cream "lit readout" color for instrument *values* - warm off-white reads as
/// a backlit digit window, not a flat white sticker.
pub(crate) const READOUT_CREAM: Color32 = Color32::from_rgb(222, 214, 184);
/// Dim engraved tone for instrument *labels* (the caption beside a value), so a
/// label recedes and its value reads as the lit element.
pub(crate) const LABEL_DIM: Color32 = Color32::from_rgb(150, 156, 150);
/// Amber accent for a section header (the title atop a cluster).
pub(crate) const HEADER_AMBER: Color32 = Color32::from_rgb(230, 178, 86);
/// Red fault-lamp tone.
pub(crate) const ACCENT_RED: Color32 = Color32::from_rgb(214, 92, 76);

/// Panel body: near-black blue-gray gunmetal, slightly translucent so the globe
/// shows faintly through the instrument cluster.
pub(crate) const PANEL_FILL: Color32 = Color32::from_rgba_unmultiplied_const(24, 28, 32, 236);
/// Recessed/inset field (egui's extreme/faint backgrounds, e.g. the slider
/// track): darker than the panel, like a cut-in readout well.
pub(crate) const RECESS_FILL: Color32 = Color32::from_rgb(12, 14, 16);
/// Raised-edge highlight painted along a panel's top/left (the lit metal lip).
pub(crate) const BEVEL_LIGHT: Color32 = Color32::from_rgb(92, 102, 110);
/// The panel outline / dark recess edge (bottom-right of the faked bevel).
pub(crate) const BEVEL_DARK: Color32 = Color32::from_rgb(8, 10, 12);

/// A key (button) at rest: brushed gunmetal, lighter than the panel body.
const KEY_FILL: Color32 = Color32::from_rgb(46, 52, 57);
/// Key outline at rest.
const KEY_EDGE: Color32 = Color32::from_rgb(80, 90, 98);
/// Key under the pointer.
const KEY_HOVER: Color32 = Color32::from_rgb(58, 66, 72);
/// Key while pressed/engaged: green-tinted, echoing the lit lamp accents.
pub(crate) const KEY_ACTIVE: Color32 = Color32::from_rgb(40, 66, 46);
/// Lamp-green accent for engaged keys and the slider grab/trail.
pub(crate) const ACCENT_GREEN: Color32 = Color32::from_rgb(122, 214, 130);
/// Screw-head metal for the corner rivets.
const RIVET_BODY: Color32 = Color32::from_rgb(96, 104, 110);
/// Screw-slot / shadow for the corner rivets.
const RIVET_SLOT: Color32 = Color32::from_rgb(20, 24, 28);

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
/// highlight is painted separately by [`paint_bevel`]), small radius, a drop
/// shadow to lift it off the globe, and a generous inner margin so the contents
/// sit inboard of the rivet line.
pub(crate) fn panel_frame() -> egui::Frame {
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

/// Paints the raised-lip highlight (top + left edges) just inside a panel's
/// outline: the lit metal lip that makes the panel read as a raised key
/// cluster, not a flat card. The dark bottom/right is the frame stroke itself.
pub(crate) fn paint_bevel(painter: &egui::Painter, rect: egui::Rect) {
    let inner = rect.shrink(1.0);
    let stroke = Stroke::new(1.0, BEVEL_LIGHT);
    painter.line_segment([inner.left_top(), inner.right_top()], stroke);
    painter.line_segment([inner.left_top(), inner.left_bottom()], stroke);
}

/// Paints a slotted screw head inset from each corner of a panel - the
/// bolted-down hardware look from the Apollo references. Each is a small metal
/// disc with a dark outline and a horizontal slot.
pub(crate) fn paint_rivets(painter: &egui::Painter, rect: egui::Rect) {
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
