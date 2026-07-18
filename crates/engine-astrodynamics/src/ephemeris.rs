//! Ephemeris lookups for the DE440 solar-system bodies, delegating to
//! satkit's `jplephem` behind a crate-owned API (glam vectors, project body
//! naming). All queries require [`crate::init`] first.

use glam::DVec3;
use satkit::{Instant, SolarSystem};

/// Solar-system bodies resolvable through the DE440 ephemeris.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    Sol,
    Mercury,
    Venus,
    /// The Terra-Luna barycenter (DE440's EMB point).
    TerraLunaBarycenter,
    Luna,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    Pluto,
}

impl Body {
    fn to_satkit(self) -> SolarSystem {
        match self {
            Body::Sol => SolarSystem::Sun,
            Body::Mercury => SolarSystem::Mercury,
            Body::Venus => SolarSystem::Venus,
            Body::TerraLunaBarycenter => SolarSystem::EMB,
            Body::Luna => SolarSystem::Moon,
            Body::Mars => SolarSystem::Mars,
            Body::Jupiter => SolarSystem::Jupiter,
            Body::Saturn => SolarSystem::Saturn,
            Body::Uranus => SolarSystem::Uranus,
            Body::Neptune => SolarSystem::Neptune,
            Body::Pluto => SolarSystem::Pluto,
        }
    }
}

/// Ephemeris lookup failure (time outside DE440 range, or unseeded data).
#[derive(Debug)]
pub struct EphemerisError(String);

impl std::fmt::Display for EphemerisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ephemeris lookup failed: {}", self.0)
    }
}

impl std::error::Error for EphemerisError {}

pub type Result<T> = core::result::Result<T, EphemerisError>;

/// GCRF position of `body` relative to Terra's center, meters.
pub fn geocentric_pos(body: Body, time: &Instant) -> Result<DVec3> {
    satkit::jplephem::geocentric_pos(body.to_satkit(), time)
        .map(vec)
        .map_err(err)
}

/// ICRF position of `body` relative to the solar-system barycenter, meters.
pub fn barycentric_pos(body: Body, time: &Instant) -> Result<DVec3> {
    satkit::jplephem::barycentric_pos(body.to_satkit(), time)
        .map(vec)
        .map_err(err)
}

/// GCRF (position m, velocity m/s) of `body` relative to Terra's center.
pub fn geocentric_state(body: Body, time: &Instant) -> Result<(DVec3, DVec3)> {
    satkit::jplephem::geocentric_state(body.to_satkit(), time)
        .map(|(pos, vel)| (vec(pos), vec(vel)))
        .map_err(err)
}

/// ICRF (position m, velocity m/s) of `body` relative to the barycenter.
pub fn barycentric_state(body: Body, time: &Instant) -> Result<(DVec3, DVec3)> {
    satkit::jplephem::barycentric_state(body.to_satkit(), time)
        .map(|(pos, vel)| (vec(pos), vec(vel)))
        .map_err(err)
}

fn vec(v: satkit::Vector3) -> DVec3 {
    DVec3::new(v[(0, 0)], v[(1, 0)], v[(2, 0)])
}

fn err(error: satkit::jplephem::Error) -> EphemerisError {
    EphemerisError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;

    const ALL: [Body; 11] = [
        Body::Sol,
        Body::Mercury,
        Body::Venus,
        Body::TerraLunaBarycenter,
        Body::Luna,
        Body::Mars,
        Body::Jupiter,
        Body::Saturn,
        Body::Uranus,
        Body::Neptune,
        Body::Pluto,
    ];

    fn epoch() -> Instant {
        Instant::from_datetime(2020, 1, 1, 0, 0, 0.0).expect("valid test epoch")
    }

    #[test]
    fn luna_geocentric_distance_within_perigee_apogee() {
        init();
        let distance = geocentric_pos(Body::Luna, &epoch())
            .expect("luna lookup")
            .length();
        assert!(
            (356_500e3..406_700e3).contains(&distance),
            "luna distance {distance} m"
        );
    }

    #[test]
    fn sol_geocentric_distance_about_one_au() {
        init();
        let distance = geocentric_pos(Body::Sol, &epoch())
            .expect("sol lookup")
            .length();
        assert!(
            (1.45e11..1.53e11).contains(&distance),
            "sol distance {distance} m"
        );
    }

    #[test]
    fn all_bodies_resolve() {
        init();
        let time = epoch();
        for body in ALL {
            geocentric_pos(body, &time).unwrap_or_else(|e| panic!("{body:?} geocentric_pos: {e}"));
            barycentric_pos(body, &time)
                .unwrap_or_else(|e| panic!("{body:?} barycentric_pos: {e}"));
            geocentric_state(body, &time)
                .unwrap_or_else(|e| panic!("{body:?} geocentric_state: {e}"));
            barycentric_state(body, &time)
                .unwrap_or_else(|e| panic!("{body:?} barycentric_state: {e}"));
        }
    }

    #[test]
    fn luna_orbital_speed_plausible() {
        init();
        let (_, vel) = geocentric_state(Body::Luna, &epoch()).expect("luna state");
        let speed = vel.length();
        assert!((900.0..1100.0).contains(&speed), "luna speed {speed} m/s");
    }
}
