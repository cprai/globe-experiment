//! The celestial-body hierarchy: the shared vocabulary for every renderable
//! body (the Earth, the Moon, and the seven planets), plus its per-frame
//! placement.
//!
//! Before this, the Moon and the planets were ad-hoc parallel cases - the Moon
//! a set of loose fields, the planets a flat list - even though both are the
//! same thing: a sun-lit body with a center and an orientation. Here they share
//! one identity type. The hierarchy groups a planet with its satellites into a
//! *system*: the Earth and Moon form the Earth system. It is built to absorb
//! the moons of other planets and Saturn's rings later (a new variant + entity
//! enum) without touching the renderer's contract.
//!
//! Identity is kept **separate from placement**: [`CelestialBody`] names a body
//! (and carries no position), so the camera target and the body selectors can
//! speak it directly; [`BodyState`] pairs that identity with a per-frame
//! [`Placement`] for the render list. Body radii are NOT stored - they come
//! from the identity via the single-source-of-truth geometry modules
//! (`earth`/`moon`/`planet`), exactly as before.

use glam::{Mat3, Vec3};

use crate::planet;
use crate::{earth, moon};

/// A member of the Earth system. The Moon is named here rather than as a
/// top-level body so the hierarchy reads as "the Earth system contains the
/// Earth and the Moon" - the abstraction that makes adding other planets' moons
/// a local change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EarthSystemEntity {
    Earth,
    Moon,
}

/// Identity of a renderable body, with no placement. This is the vocabulary the
/// [`crate::simulation::CameraTarget`] and the body selectors speak; per-frame
/// position/orientation lives in [`BodyState`].
///
/// Hierarchical: the Earth and Moon are reached through [`EarthSystemEntity`];
/// the seven planets are listed individually. The variants are ordered by
/// distance from the Sun, with the Earth system sitting at Earth's orbit
/// (between Venus and Mars). A future `SaturnSystem(SaturnSystemEntity)` would
/// group Saturn + its rings the same way the Earth system groups the Earth + the
/// Moon. The planet-specific data (oblate radii, IAU rotation, texture) hangs
/// off these variants in [`crate::planet`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CelestialBody {
    Mercury,
    Venus,
    EarthSystem(EarthSystemEntity),
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
}

impl CelestialBody {
    /// The Earth (shorthand for the most common identity).
    pub const EARTH: CelestialBody = CelestialBody::EarthSystem(EarthSystemEntity::Earth);
    /// The Moon.
    pub const MOON: CelestialBody = CelestialBody::EarthSystem(EarthSystemEntity::Moon);

    /// Display name (e.g. "Earth", "Moon", "Jupiter"), for the body-selector
    /// keys.
    pub fn name(self) -> &'static str {
        match self {
            CelestialBody::EarthSystem(EarthSystemEntity::Earth) => "Earth",
            CelestialBody::EarthSystem(EarthSystemEntity::Moon) => "Moon",
            CelestialBody::Mercury => "Mercury",
            CelestialBody::Venus => "Venus",
            CelestialBody::Mars => "Mars",
            CelestialBody::Jupiter => "Jupiter",
            CelestialBody::Saturn => "Saturn",
            CelestialBody::Uranus => "Uranus",
            CelestialBody::Neptune => "Neptune",
        }
    }

    /// Characteristic mean radius (km). The camera scales its distance/zoom
    /// limits, near plane, and pan rate by this so the interaction feel is the
    /// same fraction of the body whichever one is targeted.
    pub fn mean_radius_km(self) -> f32 {
        match self {
            CelestialBody::EarthSystem(EarthSystemEntity::Earth) => earth::MEAN_RADIUS_KM,
            CelestialBody::EarthSystem(EarthSystemEntity::Moon) => moon::MEAN_RADIUS_KM,
            // Planets: IAU volumetric-ish mean radius from the oblate axes.
            _ => (2.0 * self.equatorial_radius_km() + self.polar_radius_km()) / 3.0,
        }
    }

    /// Look-at anchor on the body surface at `(lat, lon)` (radians), in the
    /// body frame (km). Delegates to the single-source-of-truth geometry
    /// modules (`earth`/`moon`) or the oblate-planet geometry in `planet`.
    pub fn surface_position(self, latitude: f32, longitude: f32) -> Vec3 {
        match self {
            CelestialBody::EarthSystem(EarthSystemEntity::Earth) => {
                earth::surface_position(latitude, longitude)
            }
            CelestialBody::EarthSystem(EarthSystemEntity::Moon) => {
                moon::surface_position(latitude, longitude)
            }
            _ => planet::surface_position(self, latitude, longitude),
        }
    }

    /// Outward unit normal of the body surface at `(lat, lon)` (radians), in
    /// the body frame - the local "up" the eye offsets along.
    pub fn geodetic_normal(self, latitude: f32, longitude: f32) -> Vec3 {
        match self {
            CelestialBody::EarthSystem(EarthSystemEntity::Earth) => {
                earth::geodetic_normal(latitude, longitude)
            }
            CelestialBody::EarthSystem(EarthSystemEntity::Moon) => {
                moon::geodetic_normal(latitude, longitude)
            }
            _ => planet::geodetic_normal(self, latitude, longitude),
        }
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
/// per-planet `PlanetState` and the three loose Moon fields.
#[derive(Clone, Copy, Debug)]
pub struct BodyState {
    pub body: CelestialBody,
    pub placement: Placement,
}
