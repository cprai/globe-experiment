//! The cannonball spacecraft model (spec §4.3): a sphere presents the same
//! cross-section from every direction, so no attitude state exists
//! anywhere in the propagator. Shared by SRP and drag (P5/P6); estimable
//! `c_r`/`c_d` absorb the shape error this simplification introduces.

use glam::DVec3;

/// Physical spacecraft parameters for the non-gravitational forces. A
/// scene supplies one per body - or none, which skips SRP/drag/albedo
/// entirely (the parameter-less tracked-satellite behavior).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacecraftModel {
    pub mass_kg: f64,
    /// Cannonball radius; area = pi r^2 from every direction.
    pub radius_m: f64,
    /// SRP reflectivity coefficient, [1, 2] (1 = perfect absorber).
    pub c_r: f64,
    /// Drag coefficient (~2.2 for spheres in free-molecular flow).
    pub c_d: f64,
}

impl SpacecraftModel {
    /// Projected area toward `direction`, m^2. The cannonball ignores the
    /// argument - the direction-taking signature is deliberate (spec §4.3,
    /// owner decision §9-Q3b): a future attitude-dependent projected-area
    /// model (wgpu-computed) replaces the sphere behind exactly this
    /// interface.
    pub fn area_m2(&self, _direction: DVec3) -> f64 {
        std::f64::consts::PI * self.radius_m * self.radius_m
    }
}
