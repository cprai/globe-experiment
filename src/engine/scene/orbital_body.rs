//! SGP4-tracked body: TLE parse + satkit SGP4, resolved into the renderer's
//! world frame (km). TLE literals live in the scenes; this module propagates
//! whatever it is handed.
//!
//! Dot chain: SGP4 (TEME, m) -> ITRF via the full `qteme2itrf` (reads the
//! EOP table pre-seeded in `celestial_sphere::init_satkit`) -> geodetic ->
//! world via the project WGS84 helpers, so the dot lands on exactly the
//! ellipsoid the Terra impostor traces.

use glam::DVec3;
use satkit::frametransform::{qteme2gcrf, qteme2itrf};
use satkit::sgp4::sgp4;
use satkit::tle::TLE;
use satkit::{Instant, Vector3};

use crate::engine::scene::body::{
    BodyState, OrbitState, TRAIL_SEGMENTS, path_sample_times, state_from_itrf, world_km_from_itrf_m,
};

/// A body tracked from its TLE. Holds only the element set and name; state
/// is derived on demand via [`state_at`](Self::state_at).
pub struct OrbitalBody {
    /// `&mut` is needed to propagate (satkit's `sgp4` caches its
    /// initialization in the TLE on first call), so propagation methods take
    /// `&mut self`.
    tle: TLE,
    /// Object name from the TLE (e.g. "ISS (ZARYA)").
    pub name: String,
}

impl OrbitalBody {
    /// Parses a 3-line TLE (name line + two element lines). Panics on
    /// malformed input - TLEs are inline source literals, so a failure is a
    /// build-time bug, handled like the other embedded data.
    pub fn from_tle(tle_3line: &str) -> Self {
        let mut lines = tle_3line.lines();
        let line0 = lines.next().expect("TLE line 0 (name)");
        let line1 = lines.next().expect("TLE line 1");
        let line2 = lines.next().expect("TLE line 2");

        let tle = TLE::load_3line(line0, line1, line2).expect("parse embedded TLE");
        let name = tle.name.clone();
        Self { tle, name }
    }

    /// The TLE's epoch - the simulation clock's natural starting time.
    pub fn epoch(&self) -> Instant {
        self.tle.epoch
    }

    /// Propagates to `time`, in the world frame. `&mut self` only because
    /// satkit's `sgp4` caches initialization in the TLE; nothing is stored.
    pub fn state_at(&mut self, time: &Instant) -> BodyState {
        // SGP4 -> TEME position + velocity (m, m/s); one time sample, so the
        // 3xN matrices have a single column.
        let sgp4_state = sgp4(&mut self.tle, &[*time]).expect("sgp4 propagation");
        let teme = Vector3::new([
            [sgp4_state.pos[(0, 0)]],
            [sgp4_state.pos[(1, 0)]],
            [sgp4_state.pos[(2, 0)]],
        ]);
        let teme_vel = Vector3::new([
            [sgp4_state.vel[(0, 0)]],
            [sgp4_state.vel[(1, 0)]],
            [sgp4_state.vel[(2, 0)]],
        ]);

        // TEME -> GCRF. Rotating the velocity by the same quaternion as the
        // position is correct: both frames are quasi-inertial, so there is no
        // omega-cross term (unlike a rotation into the Earth-fixed ITRF).
        let q_gcrf = qteme2gcrf(time);
        let pos_gcrf = q_gcrf * teme;
        let vel_gcrf = q_gcrf * teme_vel;

        // TEME -> ITRF (Earth-fixed), then to geodetic lat/lon/height.
        let itrf = qteme2itrf(time) * teme;

        state_from_itrf(
            &itrf,
            OrbitState {
                pos_gcrf_m: DVec3::new(pos_gcrf[0], pos_gcrf[1], pos_gcrf[2]),
                vel_gcrf_m_s: DVec3::new(vel_gcrf[0], vel_gcrf[1], vel_gcrf[2]),
            },
        )
    }

    /// Predicted trail: one batch SGP4 call over one period from `time`;
    /// period from the element set's mean motion. Frame treatment doc:
    /// `TrackedBody::trail`.
    pub fn trail(&mut self, time: &Instant) -> Vec<DVec3> {
        // TLE mean motion is revolutions per day.
        let period_s = 86_400.0 / self.tle.mean_motion;
        let times = path_sample_times(time, period_s, TRAIL_SEGMENTS);
        let state = sgp4(&mut self.tle, &times).expect("sgp4 trail propagation");

        let q = qteme2itrf(time);
        (0..=TRAIL_SEGMENTS)
            .map(|i| {
                let teme = Vector3::new([
                    [state.pos[(0, i)]],
                    [state.pos[(1, i)]],
                    [state.pos[(2, i)]],
                ]);
                world_km_from_itrf_m(&(q * teme))
            })
            .collect()
    }
}

/// A scene's SGP4-tracked bodies, supplied per scene by
/// `#[derive(SceneOrbitalBodies)]` - an empty slice when the scene has no
/// `orbital_bodies` field. `&mut` because propagation caches into the TLE.
pub trait SceneOrbitalBodies {
    fn orbital_bodies_mut(&mut self) -> &mut [OrbitalBody];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test fixture ONLY - production TLEs are inline consts in the scenes
    /// (`code-style.md`). The ISS element set, epoch 2024-001.5.
    const TEST_TLE: &str = concat!(
        "ISS (ZARYA)\n",
        "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
        "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
    );

    /// The SGP4 pipeline must hold the ISS LEO: the resolved state sits in
    /// the real altitude band under its inclination, and the trail is a
    /// near-closed one-period ellipse of the right radius. Loose km-scale
    /// tolerances absorb J2 nodal/apsidal drift over the period; a frame or
    /// unit mix-up misses by orders of magnitude.
    #[test]
    fn orbital_body_holds_leo() {
        crate::engine::scene::celestial_sphere::init_satkit_for_tests();

        let mut body = OrbitalBody::from_tle(TEST_TLE);
        assert_eq!(body.name, "ISS (ZARYA)");
        let epoch = body.epoch();

        let state = body.state_at(&epoch);
        assert!(
            state.altitude_km > 300.0 && state.altitude_km < 500.0,
            "ISS altitude {:.1} km outside its LEO band",
            state.altitude_km
        );
        assert!(
            state.latitude_deg.abs() <= 51.7,
            "latitude {:.2} exceeds the 51.64 deg inclination",
            state.latitude_deg
        );

        let trail = body.trail(&epoch);
        assert_eq!(trail.len(), TRAIL_SEGMENTS + 1);
        assert!(
            (trail[0].length() - state.position_km.length()).abs() < 5.0,
            "trail start radius {:.1} km off the dot's {:.1} km",
            trail[0].length(),
            state.position_km.length()
        );
        for point in &trail {
            let r = point.length();
            assert!(
                (6_600.0..7_000.0).contains(&r),
                "trail sample at {r:.1} km left the LEO band"
            );
        }
        assert!(
            (trail[TRAIL_SEGMENTS] - trail[0]).length() < 100.0,
            "one-period trail failed to nearly close: gap {:.1} km",
            (trail[TRAIL_SEGMENTS] - trail[0]).length()
        );
    }
}
