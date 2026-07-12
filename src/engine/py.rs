//! Embedded Python for scene scripting: interpreter init, the `globe`
//! module, and the runtime script loader (scripts are re-read every launch -
//! edit + relaunch, no rebuild). The headless binary compiles but never
//! calls this: it links libpython without initializing the interpreter.

use std::ffi::CString;
use std::path::Path;
use std::sync::Once;

use pyo3::prelude::*;
use pyo3::types::PyModule;
use pyo3::{PyClass, PyTypeInfo};

/// The Python-visible module name - THE definition. The `#[pymodule]` fn
/// ident below must match: pyo3 derives the import name from the ident and
/// takes only literals, so the two cannot share one token.
pub const MODULE_NAME: &str = "globe";

/// Stamps `T.__module__` = [`MODULE_NAME`]. `#[pyclass(module = ...)]`
/// takes only string literals, so the name is stamped at runtime to keep it
/// defined once. Registration below covers every module class; call this
/// directly only for pyclasses that reach Python unregistered (the `*_py`
/// scene Inners).
pub fn set_class_module<T: PyTypeInfo>(py: Python<'_>) -> PyResult<()> {
    T::type_object(py).setattr("__module__", MODULE_NAME)
}

/// The embedded `globe` module - the Python half of the dual Rust/Python UI
/// API. The `*_py` scene structs are pyclasses too but are NOT registered
/// here: they live in `src/scenes/` (which engine must not reference) and
/// reach Python only as instances handed into the script.
#[pymodule]
fn globe(m: &Bound<'_, PyModule>) -> PyResult<()> {
    use crate::engine::scene;
    use crate::engine::ui;

    fn add<T: PyClass>(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<T>()?;
        set_class_module::<T>(m.py())
    }

    add::<ui::Header>(m)?;
    add::<ui::Readout>(m)?;
    add::<ui::DualReadout>(m)?;
    add::<ui::Button>(m)?;
    add::<ui::Toggle>(m)?;
    add::<ui::Lamp>(m)?;
    add::<ui::LampStatus>(m)?;
    add::<ui::Slider>(m)?;
    add::<ui::PanelAnchor>(m)?;
    add::<ui::py::Panel>(m)?;
    add::<ui::py::InteractiveButton>(m)?;
    add::<ui::py::InteractiveHoldButton>(m)?;
    add::<ui::py::InteractiveToggle>(m)?;
    add::<ui::py::InteractiveSlider>(m)?;
    add::<scene::SatelliteTelemetry>(m)?;
    add::<scene::satellite::OrbitShape>(m)?;
    Ok(())
}

/// Initializes the embedded interpreter, exactly once per process.
///
/// `append_to_inittab` MUST precede `Python::initialize()` - a module
/// appended after init would never be importable - so both live inside the
/// one `Once`. Call before any `Python::attach` (there is no
/// auto-initialize; an early attach panics loudly, the desired failure).
pub fn init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        pyo3::append_to_inittab!(globe);
        Python::initialize();
    });
}

/// Loads the scene script at `path` (caller-chosen, no resolution here) and
/// returns its module-level `get_drawables`. Failures panic with the Python
/// traceback printed: the script loads once at startup, so fail-fast beats
/// limping into a per-frame error loop.
pub fn load_get_drawables(py: Python<'_>, path: &Path) -> Py<PyAny> {
    let source = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    let source = CString::new(source)
        .unwrap_or_else(|_| panic!("{} contains an interior NUL byte", path.display()));
    let file_name_c = CString::new(path.display().to_string()).expect("path with NUL");

    let module =
        PyModule::from_code(py, &source, &file_name_c, c"scene_script").unwrap_or_else(|e| {
            e.print(py);
            panic!("failed to compile {}", path.display());
        });
    module
        .getattr("get_drawables")
        .unwrap_or_else(|e| {
            e.print(py);
            panic!("{} defines no get_drawables(scene)", path.display());
        })
        .unbind()
}
