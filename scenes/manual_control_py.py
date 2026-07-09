# UI-panel producer for the `manual_control_py` scene - the Python twin of
# the panel builder in `src/scenes/manual_control.rs` (kept side by side so
# the two APIs can be compared line for line). Loaded at scene startup and
# re-read on every launch: edit this file and relaunch, no rebuild.
#
# Contract: define `get_drawables(scene) -> list[Panel]`. It runs every
# frame with the live scene object (`globe.ManualControlScene`); panel
# callbacks fire later in the same frame, when egui's hit test triggers
# them - never call them yourself from here.

import math

from globe import (
    Button,
    Clock,
    DualReadout,
    Header,
    InteractiveHoldButton,
    InteractiveSlider,
    InteractiveToggle,
    Panel,
    PanelAnchor,
    Readout,
    Slider,
    Toggle,
)


def time_panel(scene):
    """The top-left Time panel: UTC + speed readouts, Run key, speed slider."""
    clock = scene.clock  # the live shared clock - callbacks mutate it below

    def toggle_run():
        clock.paused = not clock.paused

    def set_speed(exponent):
        # Exponential (base e) speed: the slider edits the exponent, so
        # multiplier = e^exp - real time at the left, 100x at the right.
        clock.multiplier = math.exp(exponent)

    rows = [
        [Header("Time")],
        [Readout("UTC", clock.datetime_label())],
        [
            # Padded to the widest value ("100.0" = 5 chars, monospace) so
            # the digit window keeps its size as the speed changes.
            Readout("Speed", "%5.1f" % clock.multiplier, "x"),
            InteractiveToggle(Toggle("Run", not clock.paused), toggle_run),
        ],
        [
            InteractiveSlider(
                Slider(
                    math.log(clock.multiplier),
                    (math.log(Clock.MIN_MULTIPLIER), math.log(Clock.MAX_MULTIPLIER)),
                ),
                set_speed,
            )
        ],
    ]
    return Panel(PanelAnchor.TopLeft, rows)


def telemetry_panel(scene):
    """The top-right readouts: lat/lon, alt + speed, apo/peri (dashes on an
    escape orbit, which has no apsides)."""
    telemetry = scene.telemetry()
    shape = scene.orbit_shape()
    if shape is not None:
        apo = "%7.1f" % shape.apoapsis_alt_km
        peri = "%7.1f" % shape.periapsis_alt_km
        speed = "%7.1f" % shape.speed_m_s
    else:
        apo = "%7s" % "---"
        peri = "%7s" % "---"
        speed = "%7.1f" % scene.speed_m_s()

    rows = [
        [Header(scene.name)],
        [
            DualReadout(
                "Lat", "%7.2f" % telemetry.latitude_deg, "deg",
                "Lon", "%7.2f" % telemetry.longitude_deg, "deg",
            )
        ],
        [
            Readout("Alt", "%6.1f" % telemetry.altitude_km, "km"),
            Readout("Speed", speed, "m/s"),
        ],
        [DualReadout("Apo", apo, "km", "Peri", peri, "km")],
    ]
    return Panel(PanelAnchor.TopRight, rows)


def burns_panel(scene):
    """The bottom-center Burns panel: six hold-to-fire keys in opposing
    pairs. The scene's bound request methods are the hold callbacks - they
    fire every frame a key is held, and the Rust side folds the requests
    into thrust."""

    def key(label, on_hold):
        return InteractiveHoldButton(Button(label), on_hold)

    rows = [
        [Header("Burns")],
        [key("Prograde", scene.request_prograde), key("Retrograde", scene.request_retrograde)],
        [key("Normal", scene.request_normal), key("Anti-Normal", scene.request_anti_normal)],
        [key("Radial Out", scene.request_radial_out), key("Radial In", scene.request_radial_in)],
    ]
    return Panel(PanelAnchor.BottomCenter, rows)


def get_drawables(scene):
    return [time_panel(scene), telemetry_panel(scene), burns_panel(scene)]
