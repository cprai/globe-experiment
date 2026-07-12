# UI-panel producer for the `solar_system_py` scene (its required --script
# path; edit + relaunch, no rebuild). Contract: get_drawables(scene) ->
# list[Panel], run every frame; panel callbacks fire later in the same frame.

import math

from globe import (
    Clock,
    Header,
    InteractiveSlider,
    InteractiveToggle,
    Panel,
    PanelAnchor,
    Readout,
    Slider,
    Toggle,
)


def time_panel(scene):
    """Top-left Time panel: UTC + speed readouts, Run key, speed slider.
    Deliberately duplicated from manual_control_py so each script can
    diverge."""

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
                    (math.log(Clock.MIN_MULTIPLIER), math.log(Clock.MAX_MULTIPLIER)),
                ),
                set_speed,
            )
        ],
    ]
    return Panel(PanelAnchor.TopLeft, rows)


def target_panel(scene):
    """Top-right Camera Target panel: one latching key per body (the orbited
    one lit); the scene folds a requested body into its camera target next
    frame."""
    selected = scene.selected_body
    rows = [[Header("Camera Target")]]
    for i, name in enumerate(scene.body_names()):
        # `i=i` pins the loop index: a bare `lambda: scene.request_body(i)`
        # would late-bind i and every key would request Neptune.
        callback = lambda i=i: scene.request_body(i)
        rows.append([InteractiveToggle(Toggle(name, selected == i), callback)])
    return Panel(PanelAnchor.TopRight, rows)


def get_drawables(scene):
    return [time_panel(scene), target_panel(scene)]
