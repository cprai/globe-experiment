//! Python face of the panel API: the [`Panel`] pyclass a scene script
//! returns from `get_drawables(scene)`, the `Interactive*` twins that pair a
//! bare instrument with a Python callable (the script-side counterparts of
//! the closure-holding Rust wrappers - deliberately the same names, referred
//! to by `super::` path here), and [`panels_from_python`], the per-frame
//! conversion into the renderer-facing [`UIDrawablePanel`]s.
//!
//! The bare instruments need no twin: they are pyclasses themselves (see the
//! instrument files), so a script builds the very same structs the Rust
//! scenes do and the conversion clones them out of their pyclass cells -
//! exactly the inert path `spec.rs` takes for a deserialized mock panel.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

use super::{
    Button, DualReadout, Header, Instrument, Lamp, PanelAnchor, Readout, Slider, Toggle,
    UIDrawablePanel,
};

/// One anchored panel built by a scene script: the Python twin of
/// [`UIDrawablePanel`]. `rows` holds the script's instrument objects
/// untouched (outer list = top-to-bottom rows, inner = left-to-right
/// instruments, same as the Rust type); they are only cast and converted
/// when the frame's [`panels_from_python`] runs.
#[pyclass(module = "globe")]
pub struct Panel {
    pub anchor: PanelAnchor,
    pub rows: Vec<Vec<Py<PyAny>>>,
}

#[pymethods]
impl Panel {
    #[new]
    fn py_new(anchor: PanelAnchor, rows: Vec<Vec<Py<PyAny>>>) -> Self {
        Self { anchor, rows }
    }
}

/// A [`Button`] paired with a Python `on_press` callable, fired on click -
/// the script-side [`super::InteractiveButton`].
#[pyclass(module = "globe", name = "InteractiveButton")]
pub struct InteractiveButton {
    pub button: Button,
    pub on_press: Py<PyAny>,
}

#[pymethods]
impl InteractiveButton {
    #[new]
    fn py_new(button: Button, on_press: Py<PyAny>) -> Self {
        Self { button, on_press }
    }
}

/// A [`Button`] paired with a Python `on_hold` callable, fired every frame
/// the key is held - the script-side [`super::InteractiveHoldButton`] (the
/// burn keys).
#[pyclass(module = "globe", name = "InteractiveHoldButton")]
pub struct InteractiveHoldButton {
    pub button: Button,
    pub on_hold: Py<PyAny>,
}

#[pymethods]
impl InteractiveHoldButton {
    #[new]
    fn py_new(button: Button, on_hold: Py<PyAny>) -> Self {
        Self { button, on_hold }
    }
}

/// A [`Toggle`] paired with a Python `on_toggle` callable, fired on click -
/// the script-side [`super::InteractiveToggle`].
#[pyclass(module = "globe", name = "InteractiveToggle")]
pub struct InteractiveToggle {
    pub toggle: Toggle,
    pub on_toggle: Py<PyAny>,
}

#[pymethods]
impl InteractiveToggle {
    #[new]
    fn py_new(toggle: Toggle, on_toggle: Py<PyAny>) -> Self {
        Self { toggle, on_toggle }
    }
}

/// A [`Slider`] paired with a Python `on_change` callable, called with the
/// new `float` on each edit - the script-side [`super::InteractiveSlider`].
#[pyclass(module = "globe", name = "InteractiveSlider")]
pub struct InteractiveSlider {
    pub slider: Slider,
    pub on_change: Py<PyAny>,
}

#[pymethods]
impl InteractiveSlider {
    #[new]
    fn py_new(slider: Slider, on_change: Py<PyAny>) -> Self {
        Self { slider, on_change }
    }
}

/// Calls a no-argument Python callback from inside an instrument's render,
/// printing (not propagating) an exception: a failed callback is one missed
/// state mutation, and panicking mid-egui-pass would unwind through the
/// presenter - unlike a failed `get_drawables`, which the scene fail-fasts
/// on (see the scene wrappers). Each call attaches on its own: callbacks
/// fire during `control_panel`'s render, outside any attach scope or pyclass
/// borrow.
fn call0_print_err(callback: &Py<PyAny>) {
    Python::attach(|py| {
        if let Err(err) = callback.call0(py) {
            err.print(py);
        }
    });
}

/// [`call0_print_err`], but passing the slider's new value.
fn call1_print_err(callback: &Py<PyAny>, value: f32) {
    Python::attach(|py| {
        if let Err(err) = callback.call1(py, (value,)) {
            err.print(py);
        }
    });
}

/// Converts a script's `get_drawables` return - any iterable of [`Panel`]s -
/// into the owned panels `control_panel` renders. Generic over the scene
/// type `S` only to satisfy the panel type: the converted callbacks ignore
/// the `&mut S` argument (the script drives the scene through its own bound
/// pymethods instead) and own `Py` handles into the interpreter (dropping
/// them without the GIL is safe - pyo3 defers the refcount decrement to the
/// next attach).
pub fn panels_from_python<S: 'static>(
    py: Python<'_>,
    panels: &Bound<'_, PyAny>,
) -> PyResult<Vec<UIDrawablePanel<S>>> {
    let mut out = Vec::new();
    for panel in panels.try_iter()? {
        let panel = panel?;
        let panel = panel.cast::<Panel>().map_err(|_| {
            PyTypeError::new_err(format!(
                "get_drawables must return Panel objects, got {}",
                type_name(&panel)
            ))
        })?;
        let panel = panel.borrow();
        let mut rows = Vec::with_capacity(panel.rows.len());
        for row in &panel.rows {
            let mut instruments: Vec<Box<dyn Instrument<S>>> = Vec::with_capacity(row.len());
            for element in row {
                instruments.push(instrument_from_python(element.bind(py))?);
            }
            rows.push(instruments);
        }
        out.push(UIDrawablePanel {
            anchor: panel.anchor,
            rows,
        });
    }
    Ok(out)
}

/// One panel element: a cast chain over every concrete instrument
/// pyclass. A bare instrument clones out as its inert self (it impls
/// [`Instrument`] for every `S`); an `Interactive*` twin becomes the Rust
/// wrapper with a GIL-acquiring closure around its Python callable (the
/// closure ignores the `&mut S` fire-time argument - the script's callback
/// reaches the scene through its own bound pymethods). Anything else is
/// a `TypeError` (propagated - a wrong element type is a script bug, handled
/// by the scene's fail-fast).
fn instrument_from_python<S: 'static>(obj: &Bound<'_, PyAny>) -> PyResult<Box<dyn Instrument<S>>> {
    let py = obj.py();
    if let Ok(cell) = obj.cast::<Header>() {
        return Ok(Box::new(cell.borrow().clone()));
    }
    if let Ok(cell) = obj.cast::<Readout>() {
        return Ok(Box::new(cell.borrow().clone()));
    }
    if let Ok(cell) = obj.cast::<DualReadout>() {
        return Ok(Box::new(cell.borrow().clone()));
    }
    if let Ok(cell) = obj.cast::<Button>() {
        return Ok(Box::new(cell.borrow().clone()));
    }
    if let Ok(cell) = obj.cast::<Toggle>() {
        return Ok(Box::new(cell.borrow().clone()));
    }
    if let Ok(cell) = obj.cast::<Lamp>() {
        return Ok(Box::new(cell.borrow().clone()));
    }
    if let Ok(cell) = obj.cast::<Slider>() {
        return Ok(Box::new(cell.borrow().clone()));
    }
    if let Ok(cell) = obj.cast::<InteractiveButton>() {
        let twin = cell.borrow();
        let callback = twin.on_press.clone_ref(py);
        return Ok(Box::new(super::InteractiveButton {
            button: twin.button.clone(),
            on_press: Box::new(move |_scene: &mut S| call0_print_err(&callback)),
        }));
    }
    if let Ok(cell) = obj.cast::<InteractiveHoldButton>() {
        let twin = cell.borrow();
        let callback = twin.on_hold.clone_ref(py);
        return Ok(Box::new(super::InteractiveHoldButton {
            button: twin.button.clone(),
            on_hold: Box::new(move |_scene: &mut S| call0_print_err(&callback)),
        }));
    }
    if let Ok(cell) = obj.cast::<InteractiveToggle>() {
        let twin = cell.borrow();
        let callback = twin.on_toggle.clone_ref(py);
        return Ok(Box::new(super::InteractiveToggle {
            toggle: twin.toggle.clone(),
            on_toggle: Box::new(move |_scene: &mut S| call0_print_err(&callback)),
        }));
    }
    if let Ok(cell) = obj.cast::<InteractiveSlider>() {
        let twin = cell.borrow();
        let callback = twin.on_change.clone_ref(py);
        return Ok(Box::new(super::InteractiveSlider {
            slider: twin.slider.clone(),
            on_change: Box::new(move |_scene: &mut S, value| call1_print_err(&callback, value)),
        }));
    }
    Err(PyTypeError::new_err(format!(
        "not a panel instrument: {}",
        type_name(obj)
    )))
}

/// The object's Python type name, best-effort (error messages only).
fn type_name(obj: &Bound<'_, PyAny>) -> String {
    obj.get_type()
        .name()
        .map(|name| name.to_string())
        .unwrap_or_else(|_| "<unknown>".to_string())
}

#[cfg(test)]
mod tests {
    use pyo3::types::PyModule;

    use super::*;

    /// A script exercising every registered class: one panel of each bare
    /// instrument, one of each interactive twin, plus a deliberately bad
    /// element for the error path.
    const SCRIPT: &std::ffi::CStr = cr#"
from globe import (Button, DualReadout, Header, InteractiveButton,
                   InteractiveHoldButton, InteractiveSlider, InteractiveToggle,
                   Lamp, LampStatus, Panel, PanelAnchor, Readout, Slider, Toggle)

def build():
    return [
        Panel(PanelAnchor.TopLeft, [
            [Header("All Instruments")],
            [Readout("A", "1"), Readout("B", "2", "km")],
            [DualReadout("L", "1", "u", "R", "2")],
            [Button("Key"), Toggle("Latch", True)],
            [Lamp("Signal", LampStatus.Ok), Slider(0.5, (0.0, 1.0))],
        ]),
        Panel(PanelAnchor.BottomCenter, [
            [InteractiveButton(Button("P"), lambda: None),
             InteractiveHoldButton(Button("H"), lambda: None),
             InteractiveToggle(Toggle("T", False), lambda: None),
             InteractiveSlider(Slider(0.0, (0.0, 1.0)), lambda v: None)],
        ]),
    ]

def bad():
    return [Panel(PanelAnchor.TopRight, [["not an instrument"]])]
"#;

    fn load_script<'py>(py: Python<'py>) -> Bound<'py, PyModule> {
        PyModule::from_code(py, SCRIPT, c"bridge_test.py", c"bridge_test").expect("script compiles")
    }

    /// Every registered class must survive the Python -> Rust round trip:
    /// the script builds one of each through the `globe` module and the
    /// conversion must reproduce the panel/row shape.
    #[test]
    fn converts_every_instrument_kind() {
        crate::engine::py::init();
        Python::attach(|py| {
            let module = load_script(py);
            let panels = module.getattr("build").unwrap().call0().unwrap();
            // Any scene type satisfies the conversion (the callbacks ignore
            // it); unit keeps the test scene-free.
            let panels = panels_from_python::<()>(py, &panels).expect("conversion succeeds");

            assert_eq!(panels.len(), 2);
            assert!(matches!(panels[0].anchor, PanelAnchor::TopLeft));
            assert!(matches!(panels[1].anchor, PanelAnchor::BottomCenter));
            let row_shape: Vec<usize> = panels[0].rows.iter().map(Vec::len).collect();
            assert_eq!(row_shape, [1, 2, 1, 2, 2]);
            assert_eq!(panels[1].rows.len(), 1);
            assert_eq!(panels[1].rows[0].len(), 4);
        });
    }

    /// A non-instrument element must fail the conversion (the scene wrapper
    /// then fail-fasts) rather than being silently dropped.
    #[test]
    fn rejects_non_instrument_elements() {
        crate::engine::py::init();
        Python::attach(|py| {
            let module = load_script(py);
            let panels = module.getattr("bad").unwrap().call0().unwrap();
            let Err(err) = panels_from_python::<()>(py, &panels) else {
                panic!("conversion of a non-instrument element must fail");
            };
            assert!(err.is_instance_of::<PyTypeError>(py));
        });
    }
}
