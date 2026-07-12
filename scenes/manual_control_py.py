# UI-panel producer for the `manual_control_py` scene (its required --script
# path; edit + relaunch, no rebuild). Contract: get_drawables(scene) ->
# list[Panel], run every frame; panel callbacks fire later in the same frame.

import math

from globe import (
    Button,
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

# Speed-slider range: real time to 100x.
MIN_MULTIPLIER = 1.0
MAX_MULTIPLIER = 100.0


def time_panel(scene):
    """Top-left Time panel: UTC + speed readouts, Run key, speed slider."""

    def toggle_run():
        scene.paused = not scene.paused

    def set_speed(exponent):
        # The slider edits the exponent (multiplier = e^exp): real time at
        # the left, 100x at the right.
        scene.multiplier = math.exp(exponent)

    rows = [
        [Header("Time")],
        [Readout("UTC", scene.datetime_label())],
        [
            # Padded to the widest value ("100.0", monospace) so the digit
            # window keeps its size as the speed changes.
            Readout("Speed", "%5.1f" % scene.multiplier, "x"),
            InteractiveToggle(Toggle("Run", not scene.paused), toggle_run),
        ],
        [
            InteractiveSlider(
                Slider(
                    math.log(scene.multiplier),
                    (math.log(MIN_MULTIPLIER), math.log(MAX_MULTIPLIER)),
                ),
                set_speed,
            )
        ],
    ]
    return Panel(PanelAnchor.TopLeft, rows)


def telemetry_panel(scene):
    """Top-right readouts: lat/lon, alt + speed, apo/peri (dashes on an
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
                "Lat",
                "%7.2f" % telemetry.latitude_deg,
                "deg",
                "Lon",
                "%7.2f" % telemetry.longitude_deg,
                "deg",
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
    """Bottom-center Burns panel: six hold-to-fire keys; the scene's bound
    request methods fire every frame a key is held."""

    def key(label, on_hold):
        return InteractiveHoldButton(Button(label), on_hold)

    rows = [
        [Header("Burns")],
        [
            key("Prograde", scene.request_prograde),
            key("Retrograde", scene.request_retrograde),
        ],
        [
            key("Normal", scene.request_normal),
            key("Anti-Normal", scene.request_anti_normal),
        ],
        [
            key("Radial Out", scene.request_radial_out),
            key("Radial In", scene.request_radial_in),
        ],
    ]
    return Panel(PanelAnchor.BottomCenter, rows)


def get_drawables(scene):
    return [time_panel(scene), telemetry_panel(scene), burns_panel(scene)]
