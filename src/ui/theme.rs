//! Apollo-panel theme: the palette, the spacing/type/radius token scales, the
//! taffy layout styles, the egui style install, and the panel chrome (frame,
//! bevel, rivets).
//!
//! The look: a rugged, dark gunmetal instrument panel floating over the bright
//! scene, with cream "lit readout" text and keys that light green when engaged.
//! References: real Apollo CSM/LM panels (gunmetal + engraved labels + lamp
//! accents) and the game UI in `ui_examples/`. Every color *and metric* is
//! theme-internal: producers (scenarios / `SimulationState`) pick *which
//! instrument* to draw, not its color or spacing - the style lives here and is
//! consumed by the instrument renderers (each owns its own
//! `Instrument::render`), and layout comes from the taffy styles below, not
//! from hand-placed pixel positions.

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke};
use egui_taffy::taffy;

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

/// Panel body: near-black blue-gray gunmetal, slightly translucent so the scene
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
/// A lit (engaged/pressed) key: solid lamp-green fill, dark engraved text -
/// the latched-key look from the game-UI reference (and Apollo's lit
/// pushbutton indicators), unmistakable against the gunmetal keys at rest.
pub(crate) const KEY_LIT: Color32 = Color32::from_rgb(74, 186, 78);
/// Engraved-dark text on a lit key (light text would wash out on the bright
/// green fill).
pub(crate) const KEY_LIT_TEXT: Color32 = Color32::from_rgb(10, 16, 10);
/// Lamp-green accent for hover strokes, the status lamp, and the slider
/// grab/trail.
pub(crate) const ACCENT_GREEN: Color32 = Color32::from_rgb(122, 214, 130);
/// Screw-head metal for the corner rivets.
const RIVET_BODY: Color32 = Color32::from_rgb(96, 104, 110);
/// Screw-slot / shadow for the corner rivets.
const RIVET_SLOT: Color32 = Color32::from_rgb(20, 24, 28);

// ---------------------------------------------------------------------------
// Metric tokens: the shared spacing / type / radius scales. Every instrument
// and layout metric derives from these - no free-floating pixel literals in
// the instruments or producers. Instruments consume them for their internal
// look; the taffy styles below consume them for inter-instrument layout.
// ---------------------------------------------------------------------------

/// Hairline weight: every stroke in the panel chrome and instruments (bevels,
/// window outlines, key edges) is one hairline - the engraved-line look.
pub(crate) const HAIRLINE: f32 = 1.0;

/// Spacing scale (egui points). XS = intra-instrument breathing room (label to
/// its window), SM = recessed-window padding, MD = value-to-unit gaps and the
/// vertical row pitch, LG = the horizontal gap between instruments in a row,
/// XL = the panel frame's inner padding, XXL = the gap between paired readouts.
pub(crate) const SPACE_XS: f32 = 2.0;
pub(crate) const SPACE_SM: f32 = 3.0;
pub(crate) const SPACE_MD: f32 = 6.0;
pub(crate) const SPACE_LG: f32 = 8.0;
pub(crate) const SPACE_XL: f32 = 12.0;
pub(crate) const SPACE_XXL: f32 = 14.0;

/// Type scale (egui points, all monospace). LABEL = dim captions and unit
/// blocks, BODY = key labels and running text, TITLE = section headers,
/// VALUE = the large digit-window values.
pub(crate) const FONT_LABEL: f32 = 11.0;
pub(crate) const FONT_BODY: f32 = 13.0;
pub(crate) const FONT_TITLE: f32 = 15.0;
pub(crate) const FONT_VALUE: f32 = 17.0;

/// Corner-radius scale: sharper the smaller the element (a stamped unit block
/// is nearly square; the panel itself is the roundest thing on screen).
pub(crate) const RADIUS_UNIT: u8 = 1;
pub(crate) const RADIUS_WINDOW: u8 = 2;
pub(crate) const RADIUS_KEY: u8 = 3;
pub(crate) const RADIUS_PANEL: u8 = 4;

/// Inset from the anchored screen corner to a panel's outer edge.
pub(crate) const PANEL_INSET: f32 = 10.0;

/// Shared minimum panel width, so a sparse panel (e.g. the body selector's
/// single key column) still reads as a panel and not a floating strip. Content
/// wider than this grows the panel; taffy owns the rest of the sizing.
pub(crate) const PANEL_MIN_WIDTH: f32 = 180.0;

// ---------------------------------------------------------------------------
// Taffy layout styles: a panel is a flex column of rows, each row a flex row
// of instrument nodes. Sizing is content-driven (plus the shared minimum
// width); no absolute positions or fixed panel boxes anywhere.
// ---------------------------------------------------------------------------

/// The panel root: a content-sized flex column of rows, stretched to a common
/// width so full-width instruments (header rule, slider) span the panel.
pub(crate) fn panel_layout() -> taffy::Style {
    taffy::Style {
        flex_direction: taffy::FlexDirection::Column,
        align_items: Some(taffy::AlignItems::Stretch),
        gap: taffy::Size {
            width: taffy::prelude::length(SPACE_LG),
            height: taffy::prelude::length(SPACE_MD),
        },
        min_size: taffy::Size {
            width: taffy::prelude::length(PANEL_MIN_WIDTH),
            height: taffy::prelude::auto(),
        },
        ..Default::default()
    }
}

/// One row of instruments: laid out horizontally, bottom-aligned so a key
/// sitting beside a digit-window readout lines up with the window (the
/// readout's caption rides above the shared baseline).
pub(crate) fn row_layout() -> taffy::Style {
    taffy::Style {
        flex_direction: taffy::FlexDirection::Row,
        align_items: Some(taffy::AlignItems::End),
        gap: taffy::Size {
            width: taffy::prelude::length(SPACE_LG),
            height: taffy::prelude::length(SPACE_MD),
        },
        ..Default::default()
    }
}

/// Installs the Apollo-panel theme onto an egui [`Context`](egui::Context):
/// monospace everywhere, the gunmetal palette, and beveled keys that light
/// green when engaged. Call once per context, right after creating it - both
/// the windowed app and the headless render path do so, so the live UI and a
/// mock overlay share one look.
pub fn install_theme(ctx: &egui::Context) {
    use egui::{FontFamily, FontId, TextStyle};

    // Two passes: egui_taffy measures content on the first pass and requests a
    // discard when the layout it drew from is stale, so the settled layout
    // needs a same-frame second pass to reach the screen.
    ctx.options_mut(|options| {
        options.max_passes = std::num::NonZeroUsize::new(2).unwrap();
    });

    let mut style = (*ctx.global_style()).clone();

    // Monospace everywhere - the instrument-readout feel. egui ships the "Hack"
    // monospace family, so this needs no font asset.
    let mono = FontFamily::Monospace;
    style.text_styles = [
        (TextStyle::Heading, FontId::new(FONT_TITLE, mono.clone())),
        (TextStyle::Body, FontId::new(FONT_BODY, mono.clone())),
        (TextStyle::Monospace, FontId::new(FONT_BODY, mono.clone())),
        (TextStyle::Button, FontId::new(FONT_BODY, mono.clone())),
        (TextStyle::Small, FontId::new(FONT_LABEL, mono)),
    ]
    .into();

    let key_radius = CornerRadius::same(RADIUS_KEY);
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = PANEL_FILL;
    v.window_fill = PANEL_FILL;
    // Inset wells (slider track, etc.) read as cut-in readout fields.
    v.extreme_bg_color = RECESS_FILL;
    v.faint_bg_color = RECESS_FILL;
    v.slider_trailing_fill = true;
    v.selection.bg_fill = ACCENT_GREEN.gamma_multiply(0.5);
    v.selection.stroke = Stroke::new(HAIRLINE, ACCENT_GREEN);

    // Widget states: brushed-gunmetal keys that light green on hover/press.
    let w = &mut v.widgets;
    let readout = Stroke::new(HAIRLINE, READOUT_CREAM);

    w.noninteractive.bg_fill = PANEL_FILL;
    w.noninteractive.weak_bg_fill = PANEL_FILL;
    w.noninteractive.bg_stroke = Stroke::new(HAIRLINE, BEVEL_DARK);
    w.noninteractive.fg_stroke = readout;
    w.noninteractive.corner_radius = key_radius;

    w.inactive.bg_fill = KEY_FILL;
    w.inactive.weak_bg_fill = KEY_FILL;
    w.inactive.bg_stroke = Stroke::new(HAIRLINE, KEY_EDGE);
    w.inactive.fg_stroke = readout;
    w.inactive.corner_radius = key_radius;

    w.hovered.bg_fill = KEY_HOVER;
    w.hovered.weak_bg_fill = KEY_HOVER;
    w.hovered.bg_stroke = Stroke::new(HAIRLINE, ACCENT_GREEN.gamma_multiply(0.8));
    w.hovered.fg_stroke = Stroke::new(HAIRLINE, ACCENT_GREEN);
    w.hovered.corner_radius = key_radius;
    w.hovered.expansion = 1.0;

    w.active.bg_fill = KEY_LIT;
    w.active.weak_bg_fill = KEY_LIT;
    w.active.bg_stroke = Stroke::new(HAIRLINE, ACCENT_GREEN);
    w.active.fg_stroke = Stroke::new(HAIRLINE, KEY_LIT_TEXT);
    w.active.corner_radius = key_radius;
    w.active.expansion = 1.0;

    w.open = w.active;

    style.spacing.item_spacing = egui::vec2(SPACE_LG, SPACE_MD);
    style.spacing.button_padding = egui::vec2(SPACE_LG, SPACE_XS * 2.0);

    ctx.set_global_style(style);
}

/// The gunmetal panel frame: dark fill, a dark outline (the raised lip's
/// highlight is painted separately by [`paint_bevel`]), small radius, a drop
/// shadow to lift it off the scene, and a generous inner margin so the contents
/// sit inboard of the rivet line.
pub(crate) fn panel_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL_FILL)
        .stroke(Stroke::new(HAIRLINE, BEVEL_DARK))
        .corner_radius(CornerRadius::same(RADIUS_PANEL))
        .inner_margin(Margin::same(SPACE_XL as i8))
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
    let stroke = Stroke::new(HAIRLINE, BEVEL_LIGHT);
    painter.line_segment([inner.left_top(), inner.right_top()], stroke);
    painter.line_segment([inner.left_top(), inner.left_bottom()], stroke);
}

/// Paints a slotted screw head inset from each corner of a panel - the
/// bolted-down hardware look from the Apollo references. Each is a small metal
/// disc with a dark outline and a horizontal slot.
pub(crate) fn paint_rivets(painter: &egui::Painter, rect: egui::Rect) {
    let inset = egui::vec2(SPACE_LG, SPACE_LG);
    // Screw-head size: between the XS and SM spacing steps - a rivet is
    // hardware, not layout, so it gets its own single tuned value.
    let radius = 2.8;
    let corners = [
        rect.left_top() + egui::vec2(inset.x, inset.y),
        rect.right_top() + egui::vec2(-inset.x, inset.y),
        rect.left_bottom() + egui::vec2(inset.x, -inset.y),
        rect.right_bottom() + egui::vec2(-inset.x, -inset.y),
    ];
    let slot = Stroke::new(HAIRLINE, RIVET_SLOT);
    for center in corners {
        painter.circle(
            center,
            radius,
            RIVET_BODY,
            Stroke::new(HAIRLINE, BEVEL_DARK),
        );
        painter.line_segment(
            [
                center + egui::vec2(-radius * 0.6, 0.0),
                center + egui::vec2(radius * 0.6, 0.0),
            ],
            slot,
        );
    }
}
