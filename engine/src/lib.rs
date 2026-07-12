//! The engine: everything used to run the app. A library crate consumed by
//! the workspace-root windowed app (`main.rs` + `scenes/`) and by this
//! package's own single-frame `headless` binary.

pub mod application;
pub mod camera;
pub mod offscreen;
pub mod planet;
pub mod py;
pub mod renderer;
pub mod scene;
pub mod ui;
