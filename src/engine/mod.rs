//! The engine: everything used to *run* the app - windowing/input, the shared
//! scene renderer, the UI instrument library, the simulation core, and the
//! body-geometry modules - as opposed to the top-level content/entry layers
//! (the bin roots, `scenarios`, and the headless `offscreen` presenter).
//!
//! Both bin roots declare this one module (`mod engine;`), so both trees
//! compile the whole engine; the headless binary simply uses the winit-free
//! subset (its crate-level `allow(dead_code)` covers the windowed-only items,
//! chiefly `application`). The two-binary separation now lives at the top
//! level only: `scenarios` exists solely in the main tree, `offscreen` solely
//! in the headless tree.

pub mod application;
pub mod camera;
pub mod planet;
pub mod renderer;
pub mod simulation;
pub mod terra;
pub mod ui;
