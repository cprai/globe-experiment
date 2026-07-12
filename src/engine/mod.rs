//! The engine: everything used to run the app. Both bin roots declare this
//! one module; the headless tree compiles the windowed-only items dead
//! (covered by its crate-level `allow(dead_code)`).

pub mod application;
pub mod camera;
pub mod planet;
pub mod py;
pub mod renderer;
pub mod scene;
pub mod ui;
