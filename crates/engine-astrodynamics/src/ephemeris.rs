//! Ephemeris lookups for the DE440 solar-system bodies over the embedded
//! anise kernels (glam vectors, project body naming), queried through the
//! caller's [`AstroData`].

use anise::constants::frames::{EARTH_J2000, SSB_J2000};
use anise::frames::Frame;
use glam::DVec3;
use hifitime::Epoch;

use crate::data::AstroData;

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
    pub(crate) fn naif_id(self) -> i32 {
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

    pub(crate) fn frame(self) -> Frame {
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
pub fn geocentric_pos(data: &AstroData, body: Body, epoch: Epoch) -> Result<DVec3> {
    state(data, body, EARTH_J2000, epoch).map(|(pos, _)| pos)
}

/// ICRF position of `body` relative to the solar-system barycenter, meters.
pub fn barycentric_pos(data: &AstroData, body: Body, epoch: Epoch) -> Result<DVec3> {
    state(data, body, SSB_J2000, epoch).map(|(pos, _)| pos)
}

/// GCRF (position m, velocity m/s) of `body` relative to Terra's center.
pub fn geocentric_state(data: &AstroData, body: Body, epoch: Epoch) -> Result<(DVec3, DVec3)> {
    state(data, body, EARTH_J2000, epoch)
}

/// ICRF (position m, velocity m/s) of `body` relative to the barycenter.
pub fn barycentric_state(data: &AstroData, body: Body, epoch: Epoch) -> Result<(DVec3, DVec3)> {
    state(data, body, SSB_J2000, epoch)
}

/// J2000-orientation (position m, velocity m/s) of `target` relative to
/// `observer` - the general form behind the geocentric/barycentric
/// specializations, for propagation segments centered on other bodies.
pub fn relative_state(
    data: &AstroData,
    target: Body,
    observer: Body,
    epoch: Epoch,
) -> Result<(DVec3, DVec3)> {
    state(data, target, observer.frame(), epoch)
}

/// Geometric (no aberration) state of `body` seen from `observer`, converted
/// km -> m at this boundary and nowhere else. anise treats the J2000
/// orientation as GCRF/ICRF, matching the satkit-era output frames.
/// Evaluated over the load-time pre-resolved segments (segments.rs) - the
/// same DE440 coefficients and interpolation code as `Almanac::translate`,
/// minus its per-call frame-tree resolution.
fn state(data: &AstroData, body: Body, observer: Frame, epoch: Epoch) -> Result<(DVec3, DVec3)> {
    data.ephemeris_segments
        .state_km(body.naif_id(), observer.ephemeris_id, epoch)
        .map(|(position, velocity)| (vec_km(position), vec_km(velocity)))
        .map_err(EphemerisError)
}

fn vec_km(v: anise::math::Vector3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z) * 1e3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::test_data;

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
        let distance = geocentric_pos(test_data(), Body::Luna, epoch())
            .expect("luna lookup")
            .length();
        assert!(
            (356_500e3..406_700e3).contains(&distance),
            "luna distance {distance} m"
        );
    }

    #[test]
    fn sol_geocentric_distance_about_one_au() {
        let distance = geocentric_pos(test_data(), Body::Sol, epoch())
            .expect("sol lookup")
            .length();
        assert!(
            (1.45e11..1.53e11).contains(&distance),
            "sol distance {distance} m"
        );
    }

    #[test]
    fn all_bodies_resolve() {
        let data = test_data();
        let time = epoch();
        for body in ALL {
            geocentric_pos(data, body, time)
                .unwrap_or_else(|e| panic!("{body:?} geocentric_pos: {e}"));
            barycentric_pos(data, body, time)
                .unwrap_or_else(|e| panic!("{body:?} barycentric_pos: {e}"));
            geocentric_state(data, body, time)
                .unwrap_or_else(|e| panic!("{body:?} geocentric_state: {e}"));
            barycentric_state(data, body, time)
                .unwrap_or_else(|e| panic!("{body:?} barycentric_state: {e}"));
        }
    }

    #[test]
    fn luna_orbital_speed_plausible() {
        let (_, vel) = geocentric_state(test_data(), Body::Luna, epoch()).expect("luna state");
        let speed = vel.length();
        assert!((900.0..1100.0).contains(&speed), "luna speed {speed} m/s");
    }
}
