//! Numerically-propagated body: a GCRF state vector + the instant it is
//! valid at, stepped by satkit `orbitprop` - no TLE behind it after seeding.
//! It re-anchors itself to each queried instant (`advance_to`, folded into
//! `state_at`/`trail`/[`KinematicBody::apply_thrust`]).

use glam::DVec3;
use satkit::frametransform::qgcrf2itrf;
use satkit::orbitprop::{self, PropSettings, SimpleState};
use satkit::{Duration, Instant, Kepler, Vector3};

use crate::engine::planet;
use crate::engine::scene::body::{
    BodyState, OrbitState, TRAIL_SEGMENTS, path_sample_times, state_from_itrf, world_km_from_itrf_m,
};
use crate::engine::scene::orbital_body::OrbitalBody;

/// A body owning its live GCRF state vector. Burns mutate its velocity, and
/// each frame's numerical propagation carries the result forward.
pub struct KinematicBody {
    /// Object name, for the panel header.
    pub name: String,
    /// THE orbit; private so a scene's only write path is
    /// [`apply_thrust`](Self::apply_thrust).
    orbit: OrbitState,
    /// The instant `orbit` is valid at; re-anchored on every query.
    epoch: Instant,
}

impl KinematicBody {
    /// Seeds ONCE from a TLE: one SGP4 sample at the TLE's own epoch,
    /// converted to a GCRF state vector (reads satkit globals -
    /// `scene::init` must have run). Numerical from then on.
    pub fn from_tle(tle_3line: &str) -> Self {
        let mut seed = OrbitalBody::from_tle(tle_3line);
        let epoch = seed.epoch();
        let orbit = seed.state_at(&epoch).orbit;
        Self {
            name: seed.name,
            orbit,
            epoch,
        }
    }

    /// The instant the state vector is valid at.
    pub fn epoch(&self) -> Instant {
        self.epoch
    }

    /// The live GCRF state - the read surface for thrust-frame bases and
    /// speed readouts.
    pub fn orbit(&self) -> OrbitState {
        self.orbit
    }

    /// Osculating apsides + speed from the state vector. Apsis radii from
    /// the Keplerian `a`/`e` (`r = a(1 +/- e)`); altitudes are above the
    /// *mean* radius (a spherical convenience readout, not the dot's
    /// geodetic WGS84 altitude). `None` for a non-elliptic (e >= 1, escape)
    /// state - no apoapsis exists; same fallback as the empty trail.
    pub fn orbit_shape(&self) -> Option<OrbitShape> {
        let (pos, vel) = self.orbit_pv();
        let kepler = Kepler::from_pv(pos, vel).ok()?;
        let mean_radius_m = planet::TERRA_MEAN_RADIUS_KM * 1000.0;
        Some(OrbitShape {
            apoapsis_alt_km: (kepler.a * (1.0 + kepler.eccen) - mean_radius_m) / 1000.0,
            periapsis_alt_km: (kepler.a * (1.0 - kepler.eccen) - mean_radius_m) / 1000.0,
            speed_m_s: self.orbit.vel_gcrf_m_s.length(),
        })
    }

    /// Thrust as one impulse: advance to `time`, then add
    /// `direction_gcrf * accel_m_s2 * dt` to the velocity, dt being the
    /// interval since the last advance (Euler integration of a continuous
    /// burn; frame dt keeps the chord error far below a game-like thrust's
    /// own fiction). dt-scaled, so a paused clock (dt <= 0) burns nothing.
    pub fn apply_thrust(&mut self, time: &Instant, direction_gcrf: DVec3, accel_m_s2: f64) {
        let dt = (*time - self.epoch).as_seconds();
        if dt <= 0.0 {
            return;
        }
        self.advance_to(time);
        self.orbit.vel_gcrf_m_s += direction_gcrf * (accel_m_s2 * dt);
    }

    /// Re-anchors the state vector to `time`: one numerical `orbitprop` step,
    /// so the stored initial conditions are always current and a burn's
    /// velocity change compounds into every later frame. No-op when `time`
    /// is not ahead of the epoch (paused clock).
    fn advance_to(&mut self, time: &Instant) {
        if (*time - self.epoch).as_seconds() <= 0.0 {
            return;
        }
        let initial = simple_state(&self.orbit);
        let result = orbitprop::propagate(&initial, &self.epoch, time, &numerical_settings(), None)
            .expect("numerical state propagation");
        let end = result.state_end;
        self.orbit = OrbitState {
            pos_gcrf_m: DVec3::new(end[0], end[1], end[2]),
            vel_gcrf_m_s: DVec3::new(end[3], end[4], end[5]),
        };
        self.epoch = *time;
    }

    /// Advances to `time`, then resolves the GCRF state to the world frame -
    /// the same [`BodyState`] the SGP4 arm produces. A repeat call at the
    /// same instant is a pure frame change.
    pub fn state_at(&mut self, time: &Instant) -> BodyState {
        self.advance_to(time);
        let gcrf = Vector3::new([
            [self.orbit.pos_gcrf_m.x],
            [self.orbit.pos_gcrf_m.y],
            [self.orbit.pos_gcrf_m.z],
        ]);
        let itrf = qgcrf2itrf(time) * gcrf;
        state_from_itrf(&itrf, self.orbit)
    }

    /// Predicted trail: one `orbitprop` propagate over one period from
    /// `time` (after advancing to it), all samples from its dense output in
    /// one `interp_batch`. EMPTY for a non-elliptic (escape) state, which
    /// has no period. Frame treatment doc: `TrackedBody::trail`.
    pub fn trail(&mut self, time: &Instant) -> Vec<DVec3> {
        self.advance_to(time);
        let (pos, vel) = self.orbit_pv();

        // Period from the osculating elements depends only on the semi-major
        // axis, so circular/equatorial singularities cannot bite. e >= 1
        // (escape, reachable by burning) has no period - return the empty
        // trail (the renderer skips it) rather than panic.
        let Ok(kepler) = Kepler::from_pv(pos, vel) else {
            return Vec::new();
        };
        let period_s = kepler.period();

        let initial = simple_state(&self.orbit);
        let settings = numerical_settings();
        let end = *time + Duration::from_seconds(period_s);
        let result = orbitprop::propagate(&initial, time, &end, &settings, None)
            .expect("numerical trail propagation");

        let times = path_sample_times(time, period_s, TRAIL_SEGMENTS);
        let samples = result
            .interp_batch(&times)
            .expect("trail dense-output sampling");

        let q = qgcrf2itrf(time);
        samples
            .iter()
            .map(|sample| {
                let gcrf = Vector3::new([[sample[0]], [sample[1]], [sample[2]]]);
                world_km_from_itrf_m(&(q * gcrf))
            })
            .collect()
    }

    /// The state vector as satkit column vectors (m, m/s).
    fn orbit_pv(&self) -> (Vector3, Vector3) {
        (
            Vector3::new([
                [self.orbit.pos_gcrf_m.x],
                [self.orbit.pos_gcrf_m.y],
                [self.orbit.pos_gcrf_m.z],
            ]),
            Vector3::new([
                [self.orbit.vel_gcrf_m_s.x],
                [self.orbit.vel_gcrf_m_s.y],
                [self.orbit.vel_gcrf_m_s.z],
            ]),
        )
    }
}

/// Osculating-orbit panel readout: apsis altitudes + current speed.
pub struct OrbitShape {
    /// Apoapsis height above Terra's mean radius, km.
    pub apoapsis_alt_km: f64,
    /// Periapsis height above Terra's mean radius, km.
    pub periapsis_alt_km: f64,
    /// Current inertial speed, m/s.
    pub speed_m_s: f64,
}

/// The [`OrbitState`] packed as satkit's 6-vector integrator state
/// (GCRF x,y,z meters + vx,vy,vz m/s).
fn simple_state(state: &OrbitState) -> SimpleState {
    let mut packed = SimpleState::zeros();
    packed[0] = state.pos_gcrf_m.x;
    packed[1] = state.pos_gcrf_m.y;
    packed[2] = state.pos_gcrf_m.z;
    packed[3] = state.vel_gcrf_m_s.x;
    packed[4] = state.vel_gcrf_m_s.y;
    packed[5] = state.vel_gcrf_m_s.z;
    packed
}

/// Shared `orbitprop` settings: defaults (EGM96 4x4, Sun/Moon third-body,
/// solid tides, relativity, adaptive RKV98 dense output). Drag/SRP only run
/// when `propagate`'s `satprops` is Some - every caller here passes None to
/// keep satkit's non-embedded space-weather loader unreachable;
/// `use_spaceweather: false` is belt-and-suspenders for the same reason.
fn numerical_settings() -> PropSettings {
    PropSettings {
        use_spaceweather: false,
        ..PropSettings::default()
    }
}

/// A scene's numerically-propagated bodies, supplied per scene by
/// `#[derive(SceneKinematicBodies)]` - an empty slice when the scene has no
/// `kinematic_bodies` field. `&mut` because each query re-anchors the state.
pub trait SceneKinematicBodies {
    fn kinematic_bodies_mut(&mut self) -> &mut [KinematicBody];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terra's GM (EGM96, m^3/s^2) - only to construct the test state's
    /// circular speed; the propagator brings its own force model.
    const MU_M3_S2: f64 = 3.986004418e14;
    /// Test orbit radius, meters (~407 km above the mean radius).
    const RADIUS_M: f64 = 6_778_000.0;

    /// Same-module struct-literal construction: no TLE behind it, like a
    /// manually-controlled body after seeding.
    fn test_body(orbit: OrbitState, epoch: Instant) -> KinematicBody {
        KinematicBody {
            name: "TEST".to_string(),
            orbit,
            epoch,
        }
    }

    /// A circular equatorial LEO state vector with no TLE behind it - the
    /// same construction a manually-controlled body lives on.
    fn circular_leo() -> (OrbitState, Instant) {
        let speed = (MU_M3_S2 / RADIUS_M).sqrt();
        let state = OrbitState {
            pos_gcrf_m: DVec3::new(RADIUS_M, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed, 0.0),
        };
        let time = Instant::from_datetime(2024, 1, 1, 12, 0, 0.0).expect("valid datetime");
        (state, time)
    }

    /// The TLE-free pipeline must hold a circular LEO: shape readout matches
    /// the constructed altitude/speed, ten minutes of propagation stays on
    /// the (near-)circle while moving along track, and the resolved dot
    /// lands on the orbit radius near the equator. Loose km-scale tolerances
    /// absorb the full force model (J2 & co.) vs the two-body construction;
    /// a frame or unit mix-up misses by orders of magnitude.
    #[test]
    fn numerical_pipeline_holds_circular_leo() {
        crate::engine::scene::celestial_sphere::init_satkit_for_tests();

        let (state, t0) = circular_leo();
        let alt_km = (RADIUS_M - planet::TERRA_MEAN_RADIUS_KM * 1000.0) / 1000.0;
        let mut body = test_body(state, t0);

        let shape = body.orbit_shape().expect("circular orbit is elliptic");
        assert!(
            (shape.apoapsis_alt_km - alt_km).abs() < 5.0,
            "apoapsis {:.1} km, expected ~{alt_km:.1}",
            shape.apoapsis_alt_km
        );
        assert!(
            (shape.periapsis_alt_km - alt_km).abs() < 5.0,
            "periapsis {:.1} km, expected ~{alt_km:.1}",
            shape.periapsis_alt_km
        );
        assert!(
            (shape.speed_m_s - state.vel_gcrf_m_s.length()).abs() < 1e-6,
            "speed readout is the state's own speed"
        );

        let t1 = t0 + Duration::from_seconds(600.0);
        body.advance_to(&t1);
        let stepped = body.orbit();
        assert!(
            (stepped.pos_gcrf_m.length() - RADIUS_M).abs() < 30_000.0,
            "radius drifted to {:.1} km",
            stepped.pos_gcrf_m.length() / 1000.0
        );
        assert!(
            (stepped.vel_gcrf_m_s.length() - shape.speed_m_s).abs() < 50.0,
            "speed drifted to {:.1} m/s",
            stepped.vel_gcrf_m_s.length()
        );
        assert!(
            (stepped.pos_gcrf_m - state.pos_gcrf_m).length() > 1_000_000.0,
            "propagation should move well along track in 600 s"
        );

        let resolved = body.state_at(&t1);
        assert!(
            (resolved.position_km.length() * 1000.0 - RADIUS_M).abs() < 30_000.0,
            "dot at {:.1} km from center",
            resolved.position_km.length()
        );
        assert!(
            resolved.latitude_deg.abs() < 1.0,
            "equatorial orbit resolved to lat {:.2}",
            resolved.latitude_deg
        );
    }

    /// Thrust semantics: at the epoch (dt = 0, the paused clock) a burn is a
    /// no-op; ahead of it, the velocity gains exactly `direction * accel *
    /// dt` on top of what coasting alone produces.
    #[test]
    fn thrust_is_dt_scaled_impulse() {
        crate::engine::scene::celestial_sphere::init_satkit_for_tests();

        let (state, t0) = circular_leo();
        let direction = DVec3::new(0.0, 1.0, 0.0);

        let mut paused = test_body(state, t0);
        paused.apply_thrust(&t0, direction, 10.0);
        assert_eq!(paused.orbit(), state, "dt = 0 must burn nothing");

        let t1 = t0 + Duration::from_seconds(10.0);
        let mut coasting = test_body(state, t0);
        coasting.advance_to(&t1);
        let mut burning = test_body(state, t0);
        burning.apply_thrust(&t1, direction, 10.0);

        let dv = burning.orbit().vel_gcrf_m_s - coasting.orbit().vel_gcrf_m_s;
        assert!(
            (dv - direction * 100.0).length() < 1e-9,
            "10 s at 10 m/s^2 must add exactly 100 m/s, got {dv:?}"
        );
        assert_eq!(
            burning.orbit().pos_gcrf_m,
            coasting.orbit().pos_gcrf_m,
            "an impulse changes velocity only"
        );
    }

    /// An escape (e >= 1) state has no apsides and no period: shape reads
    /// `None` and the trail is empty - the readout-dashes/no-line fallbacks.
    #[test]
    fn escape_orbit_has_no_shape_or_trail() {
        crate::engine::scene::celestial_sphere::init_satkit_for_tests();

        let (mut state, t0) = circular_leo();
        state.vel_gcrf_m_s *= 2.0; // well past escape velocity
        let mut body = test_body(state, t0);

        assert!(body.orbit_shape().is_none());
        assert!(body.trail(&t0).is_empty());
    }
}
