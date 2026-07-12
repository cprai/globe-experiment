//! The celestial-body hierarchy: identity ([`CelestialBody`]) kept separate
//! from per-frame placement ([`BodyState`]/[`Placement`]). A planet and its
//! satellites form a *system* (Terra + Luna today); future moon systems or
//! Saturn's rings add a variant + entity enum without touching the
//! renderer's contract. Radii are deliberately not stored - they come from
//! the identity via the shared triaxial table in `planet` (Terra's row is
//! the WGS84 ellipsoid).

use glam::{DMat3, DVec3};

use crate::planet;

/// A member of the Terra system. Luna is nested here rather than top-level
/// so adding other planets' lunas stays a local change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerraSystemEntity {
    Terra,
    Luna,
}

/// Identity of a renderable body, with no placement. Variants are ordered by
/// distance from Sol, the Terra system at Terra's orbit. Per-body data
/// (radii, IAU rotation, textures) hangs off these variants in
/// [`crate::planet`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CelestialBody {
    Mercury,
    Venus,
    TerraSystem(TerraSystemEntity),
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

impl CelestialBody {
    /// Terra (shorthand for the most common identity).
    pub const TERRA: CelestialBody = CelestialBody::TerraSystem(TerraSystemEntity::Terra);
    /// Luna.
    pub const LUNA: CelestialBody = CelestialBody::TerraSystem(TerraSystemEntity::Luna);

    /// All nine bodies, ordered by distance from Sol with Luna right after
    /// Terra - the camera-target panels' key order and the index space a
    /// `*_py` script requests target switches in (`request_body`).
    pub const ALL: [CelestialBody; 9] = [
        CelestialBody::Mercury,
        CelestialBody::Venus,
        CelestialBody::TERRA,
        CelestialBody::LUNA,
        CelestialBody::Mars,
        CelestialBody::Jupiter,
        CelestialBody::Saturn,
        CelestialBody::Uranus,
        CelestialBody::Neptune,
    ];

    /// Display name (e.g. "Terra", "Jupiter").
    pub fn name(self) -> &'static str {
        match self {
            CelestialBody::TerraSystem(TerraSystemEntity::Terra) => "Terra",
            CelestialBody::TerraSystem(TerraSystemEntity::Luna) => "Luna",
            CelestialBody::Mercury => "Mercury",
            CelestialBody::Venus => "Venus",
            CelestialBody::Mars => "Mars",
            CelestialBody::Jupiter => "Jupiter",
            CelestialBody::Saturn => "Saturn",
            CelestialBody::Uranus => "Uranus",
            CelestialBody::Neptune => "Neptune",
        }
    }

    /// Whether two DISTINCT bodies share a planetary system - the generic
    /// mutual-eclipse rule: same-system bodies shadow each other,
    /// cross-system shadows are astronomically negligible. A future system
    /// variant self-shadows by adding one arm here. A body is deliberately
    /// NOT in the same system as itself (it cannot occlude itself).
    pub fn same_system(self, other: CelestialBody) -> bool {
        if self == other {
            return false;
        }
        matches!(
            (self, other),
            (CelestialBody::TerraSystem(_), CelestialBody::TerraSystem(_))
        )
    }

    /// Surface point at `(lat, lon)` (radians), body frame (km).
    pub fn surface_position(self, latitude: f64, longitude: f64) -> DVec3 {
        planet::surface_position(self, latitude, longitude)
    }

    /// Outward unit surface normal at `(lat, lon)` (radians), body frame -
    /// the local "up".
    pub fn geodetic_normal(self, latitude: f64, longitude: f64) -> DVec3 {
        planet::geodetic_normal(self, latitude, longitude)
    }
}

/// Per-frame placement of a body: center + orientation. Radii are
/// deliberately absent - they come from the [`CelestialBody`] identity.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// Body center in the absolute world frame (heliocentric origin,
    /// Earth-fixed axes), km. **f64**: heliocentric magnitudes (~1.5e8 km to
    /// billions) overflow f32 precision when the renderer subtracts the
    /// render origin to recover a small local offset (catastrophic
    /// cancellation) - the subtraction stays f64, cast to f32 only after it
    /// lands in the small render frame.
    pub pos_world: DVec3,
    /// Body-fixed -> world rotation. Pure rotation, so it carries normals
    /// too; f64 like the position, cast to f32 only at the impostor uniform.
    pub rot: DMat3,
}

/// One renderable body for one frame: identity plus placement. Element of
/// the celestial-bodies render list.
#[derive(Clone, Copy, Debug)]
pub struct BodyState {
    pub body: CelestialBody,
    pub placement: Placement,
}
