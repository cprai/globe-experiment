//! The celestial-body hierarchy: the shared vocabulary for every renderable
//! body (Terra, Luna, and the seven planets), plus its per-frame
//! placement.
//!
//! Before this, Luna and the planets were ad-hoc parallel cases - Luna
//! a set of loose fields, the planets a flat list - even though both are the
//! same thing: a sun-lit body with a center and an orientation. Here they share
//! one identity type. The hierarchy groups a planet with its satellites into a
//! *system*: Terra and Luna form the Terra system. It is built to absorb
//! the lunas of other planets and Saturn's rings later (a new variant + entity
//! enum) without touching the renderer's contract.
//!
//! Identity is kept **separate from placement**: [`CelestialBody`] names a body
//! (and carries no position), so the camera target and the body selectors can
//! speak it directly; [`BodyState`] pairs that identity with a per-frame
//! [`Placement`] for the render list. Body radii are NOT stored - they come
//! from the identity via the single-source-of-truth geometry module (the
//! shared triaxial table in `planet`, which covers every body - Terra's row
//! is the WGS84 ellipsoid).

use glam::{Mat3, Vec3};

use crate::engine::planet;

/// A member of the Terra system. Luna is named here rather than as a
/// top-level body so the hierarchy reads as "the Terra system contains the
/// Terra and Luna" - the abstraction that makes adding other planets' lunas
/// a local change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerraSystemEntity {
    Terra,
    Luna,
}

/// Identity of a renderable body, with no placement. This is the vocabulary the
/// [`crate::engine::simulation::CameraTarget`] and the body selectors speak;
/// per-frame position/orientation lives in [`BodyState`].
///
/// Hierarchical: Terra and Luna are reached through [`TerraSystemEntity`];
/// the seven planets are listed individually. The variants are ordered by
/// distance from Sol, with the Terra system sitting at Terra's orbit
/// (between Venus and Mars). A future `SaturnSystem(SaturnSystemEntity)` would
/// group Saturn + its rings the same way the Terra system groups Terra +
/// Luna. The planet-specific data (triaxial radii, IAU rotation, texture)
/// hangs off these variants in [`crate::engine::planet`].
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

    /// Display name (e.g. "Terra", "Luna", "Jupiter"), for the body-selector
    /// keys.
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

    /// Whether two DISTINCT bodies belong to the same planetary system - the
    /// generic mutual-shadow rule. Bodies of one system eclipse each other
    /// (Terra <-> Luna today); bodies of different systems never do (their
    /// mutual shadows are astronomically negligible). The renderer builds each
    /// impostor's occluder list from this, so a future `JupiterSystem` variant
    /// self-shadows by adding one arm here, with no renderer change. A body is
    /// deliberately NOT in the same system as itself (it cannot occlude
    /// itself).
    pub fn same_system(self, other: CelestialBody) -> bool {
        if self == other {
            return false;
        }
        matches!(
            (self, other),
            (CelestialBody::TerraSystem(_), CelestialBody::TerraSystem(_))
        )
    }

    /// Look-at anchor on the body surface at `(lat, lon)` (radians), in the
    /// body frame (km). Delegates to the single-source-of-truth geometry
    /// table in `planet` (Terra's row is the WGS84 ellipsoid).
    pub fn surface_position(self, latitude: f32, longitude: f32) -> Vec3 {
        planet::surface_position(self, latitude, longitude)
    }

    /// Outward unit normal of the body surface at `(lat, lon)` (radians), in
    /// the body frame - the local "up" the eye offsets along.
    pub fn geodetic_normal(self, latitude: f32, longitude: f32) -> Vec3 {
        planet::geodetic_normal(self, latitude, longitude)
    }
}

/// Per-frame placement of a body: where its center is and how it is oriented.
/// Radii are deliberately absent - they come from the [`CelestialBody`]
/// identity (see the accessors above), so the placement stays the minimal
/// per-frame data the renderer needs.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// Body center in the absolute world (Earth-fixed ECEF) frame, km.
    pub pos_world: Vec3,
    /// Rotation taking a vector in the body-fixed frame into the world frame.
    /// Pure rotation, so it carries normals too.
    pub rot: Mat3,
}

/// One renderable body for one frame: its identity plus its placement. Element
/// of the celestial-bodies render list; the unified replacement for the old
/// per-planet `PlanetState` and the three loose Luna fields.
#[derive(Clone, Copy, Debug)]
pub struct BodyState {
    pub body: CelestialBody,
    pub placement: Placement,
}
