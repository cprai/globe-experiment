//! Scenes: each entry point pins the simulation to a specific **past** event
//! and runs the app for it. Keep every scene's time window inside the bundled
//! EOP range (1962-01-01 .. build date) - the accuracy constraint.

pub mod iss;
pub mod iss_and_hubble;
pub mod lunar_eclipse;
pub mod manual_control;
pub mod solar_eclipse;
pub mod solar_system;
