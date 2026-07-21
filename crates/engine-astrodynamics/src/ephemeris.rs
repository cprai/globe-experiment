//! Ephemeris lookups for the DE440 solar-system bodies over the embedded
//! anise kernels (glam vectors, project body naming). Queries parse the
//! kernels lazily on first touch; [`crate::init`] merely front-loads that.

use anise::constants::frames::{EARTH_J2000, SSB_J2000};
use anise::frames::Frame;
use glam::DVec3;
use hifitime::Epoch;

use crate::data::context;

/// Solar-system bodies resolvable through the DE440 ephemeris.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Body {
    Sol,
    /// Terra's center (NAIF 399) - the implicit observer of the
    /// `geocentric_*` queries, listed so it can also be a TARGET (or a
    /// third body of a non-Earth-centric propagation segment).
    Terra,
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
    /// NAIF id, same semantics as the satkit-era lookups: planet centers
    /// for Mercury/Venus (199/299), system barycenters for Mars..Pluto
    /// (4..9 - DE440 carries the outer planets only as barycenters).
    fn naif_id(self) -> i32 {
        match self {
            Body::Sol => 10,
            Body::Terra => 399,
            Body::Mercury => 199,
            Body::Venus => 299,
            Body::TerraLunaBarycenter => 3,
            Body::Luna => 301,
            Body::Mars => 4,
            Body::Jupiter => 5,
            Body::Saturn => 6,
            Body::Uranus => 7,
            Body::Neptune => 8,
            Body::Pluto => 9,
        }
    }

    fn frame(self) -> Frame {
        Frame::from_ephem_j2000(self.naif_id())
    }
}

/// Ephemeris lookup failure (time outside the embedded kernels' span).
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
pub fn geocentric_pos(body: Body, epoch: Epoch) -> Result<DVec3> {
    state(body, EARTH_J2000, epoch).map(|(pos, _)| pos)
}

/// ICRF position of `body` relative to the solar-system barycenter, meters.
pub fn barycentric_pos(body: Body, epoch: Epoch) -> Result<DVec3> {
    state(body, SSB_J2000, epoch).map(|(pos, _)| pos)
}

/// GCRF (position m, velocity m/s) of `body` relative to Terra's center.
pub fn geocentric_state(body: Body, epoch: Epoch) -> Result<(DVec3, DVec3)> {
    state(body, EARTH_J2000, epoch)
}

/// ICRF (position m, velocity m/s) of `body` relative to the barycenter.
pub fn barycentric_state(body: Body, epoch: Epoch) -> Result<(DVec3, DVec3)> {
    state(body, SSB_J2000, epoch)
}

/// J2000-orientation (position m, velocity m/s) of `target` relative to
/// `observer` - the general form behind the geocentric/barycentric
/// specializations, for propagation segments centered on other bodies.
pub fn relative_state(target: Body, observer: Body, epoch: Epoch) -> Result<(DVec3, DVec3)> {
    state(target, observer.frame(), epoch)
}

/// Geometric (no aberration) state of `body` seen from `observer`, converted
/// km -> m at this boundary and nowhere else. anise treats the J2000
/// orientation as GCRF/ICRF, matching the satkit-era output frames.
fn state(body: Body, observer: Frame, epoch: Epoch) -> Result<(DVec3, DVec3)> {
    context()
        .almanac
        .translate(body.frame(), observer, epoch, None)
        .map(|state| (vec_km(state.radius_km), vec_km(state.velocity_km_s)))
        .map_err(|error| EphemerisError(error.to_string()))
}

fn vec_km(v: anise::math::Vector3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z) * 1e3
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

    fn epoch() -> Epoch {
        Epoch::from_gregorian_utc(2020, 1, 1, 0, 0, 0, 0)
    }

    #[test]
    fn luna_geocentric_distance_within_perigee_apogee() {
        init();
        let distance = geocentric_pos(Body::Luna, epoch())
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
        let distance = geocentric_pos(Body::Sol, epoch())
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
            geocentric_pos(body, time).unwrap_or_else(|e| panic!("{body:?} geocentric_pos: {e}"));
            barycentric_pos(body, time).unwrap_or_else(|e| panic!("{body:?} barycentric_pos: {e}"));
            geocentric_state(body, time)
                .unwrap_or_else(|e| panic!("{body:?} geocentric_state: {e}"));
            barycentric_state(body, time)
                .unwrap_or_else(|e| panic!("{body:?} barycentric_state: {e}"));
        }
    }

    #[test]
    fn luna_orbital_speed_plausible() {
        init();
        let (_, vel) = geocentric_state(Body::Luna, epoch()).expect("luna state");
        let speed = vel.length();
        assert!((900.0..1100.0).contains(&speed), "luna speed {speed} m/s");
    }
}
