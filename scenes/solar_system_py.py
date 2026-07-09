# UI-panel producer for the `solar_system_py` scene - the Python twin of the
# panel builder in `src/scenes/solar_system.rs` and of
# `BodySelector::panel()` in `src/engine/scene/mod.rs` (kept side by side so
# the two APIs can be compared). Passed to the scene as its required CLI
# script path (`scene solar_system_py scenes/solar_system_py.py`), loaded at
# startup and re-read on every launch: edit this file and relaunch, no
# rebuild.
#
# Contract: define `get_drawables(scene) -> list[Panel]`. It runs every
# frame with the live scene object (`globe.SolarSystemScene`); panel
# callbacks fire later in the same frame, when egui's hit test triggers
# them - never call them yourself from here.

import math

from globe import (
    BodySelector,
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
    """The top-left Time panel: UTC + speed readouts, Run key, speed slider.
    Identical to manual_control_py's - deliberately duplicated, like the
    Rust scenes' per-scene Time panels, so each script can diverge."""
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


def selector_panel(selector):
    """The top-right camera-target selector: one latching key per body (the
    chosen one lit), a single column ordered by distance from Sol."""
    selected = selector.selected
    rows = [[Header("Camera Target")]]
    for i, name in enumerate(BodySelector.body_names()):
        # `i=i` pins the loop index: a bare `lambda: selector.request(i)`
        # would late-bind i and every key would request Neptune.
        callback = lambda i=i: selector.request(i)
        rows.append([InteractiveToggle(Toggle(name, selected == i), callback)])
    return Panel(PanelAnchor.TopRight, rows)


def get_drawables(scene):
    return [time_panel(scene), selector_panel(scene.selector)]
