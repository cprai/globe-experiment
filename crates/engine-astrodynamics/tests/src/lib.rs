//! Verification harness comparing `engine-astrodynamics` against reference
//! implementations — raw satkit today, other astrodynamics crates later.
//!
//! A test-only lib crate: `cargo test -p engine-astrodynamics-tests` runs
//! the comparisons, and `cargo bench -p engine-astrodynamics-tests --bench
//! <name>` runs one criterion benchmark target (`src/benches/*.rs` — crate
//! vs satkit timing on the same problems; omit `--bench` to run all),
//! while the reference dependencies stay out of every shipped build
//! (nothing depends on this leaf crate). Keep all sources under `src/`
//! with no `main.rs` — the parent crate auto-discovers `tests/*.rs` and
//! `tests/*/main.rs` as its own integration-test targets.
//!
//! [`data`] and [`support`] are public (not `#[cfg(test)]`) solely so the
//! bench target — a separate crate root — can reuse the seeding and time
//! bridges; nothing outside this directory consumes them.
//!
//! Initialization rule: the harness owns both sides' data. The crate side
//! is one eagerly-loaded [`engine_astrodynamics::AstroData`], shared via
//! [`data::astro`]. Seed satkit's process-wide set-once stores exclusively
//! through the `Once`-guarded [`data::seed_satkit`] (the crate itself no
//! longer touches satkit). Never call satkit's `init_*` functions
//! directly, and never combine this harness in one process with any other
//! satkit seeder (e.g. the engine's `init_satkit`) — the stores accept one
//! seeding per process.

pub mod data;

pub mod support {
    use engine_astrodynamics::Epoch;

    /// The reference-side time bridge, permanent harness infrastructure:
    /// every satkit call takes its instant from here, never from a satkit
    /// calendar constructor. Bridging through the TAI Modified Julian Date
    /// keeps both libraries on the same physical instant even where their
    /// UTC conventions differ (leap seconds, pre-1972); the f64 MJD carries
    /// ~1.3 us roundoff, orders below every comparison bound in this crate.
    pub fn satkit_instant(epoch: Epoch) -> satkit::Instant {
        satkit::Instant::from_mjd_with_scale(epoch.to_mjd_tai_days(), satkit::TimeScale::TAI)
    }

    /// Ephemeris-comparison bridge. satkit's `jplephem` evaluates the
    /// Chebyshev series at TT (`tm.as_jd_with_scale(TimeScale::TT)`,
    /// jplephem.rs), a TDB ~ TT shortcut up to +-1.7 ms off the true TDB
    /// argument - above 1e-9 relative at the Moon's geocentric angular
    /// rate, while anise converts properly. Handing the reference an
    /// instant whose TT equals our epoch's TDB makes both sides evaluate
    /// the same ephemeris argument, so the comparison stays a tight check
    /// of kernel reading, body mapping, and units - not of satkit's
    /// time-scale approximation.
    pub fn satkit_instant_at_tdb(epoch: Epoch) -> satkit::Instant {
        satkit::Instant::from_jd_with_scale(epoch.to_jde_tdb_days(), satkit::TimeScale::TT)
    }

    /// glam -> satkit vector bridge for handing both sides the same input.
    pub fn satkit_vec(v: glam::DVec3) -> satkit::Vector3 {
        satkit::Vector3::new([[v.x], [v.y], [v.z]])
    }
}

#[cfg(test)]
mod ephemeris {
    use engine_astrodynamics::Epoch;
    use engine_astrodynamics::ephemeris::{self, Body};
    use glam::DVec3;
    use satkit::SolarSystem;

    use crate::support::satkit_instant_at_tdb;

    /// Test-owned body mapping, deliberately independent of the crate's
    /// private NAIF-id table so the mapping itself is cross-checked.
    const BODIES: [(Body, SolarSystem); 11] = [
        (Body::Sol, SolarSystem::Sun),
        (Body::Mercury, SolarSystem::Mercury),
        (Body::Venus, SolarSystem::Venus),
        (Body::TerraLunaBarycenter, SolarSystem::EMB),
        (Body::Luna, SolarSystem::Moon),
        (Body::Mars, SolarSystem::Mars),
        (Body::Jupiter, SolarSystem::Jupiter),
        (Body::Saturn, SolarSystem::Saturn),
        (Body::Uranus, SolarSystem::Uranus),
        (Body::Neptune, SolarSystem::Neptune),
        (Body::Pluto, SolarSystem::Pluto),
    ];

    /// A spread of past epochs well inside DE440: 1980, J2000.0, and two
    /// arbitrary modern instants. All post-1972 on purpose — the crate and
    /// reference sides agree on these UTC labels by construction (the
    /// bridge handles the physical-instant mapping regardless).
    fn epochs() -> [Epoch; 4] {
        [
            (1980, 6, 1, 0, 0, 0),
            (2000, 1, 1, 12, 0, 0),
            (2012, 3, 15, 6, 30, 0),
            (2024, 1, 15, 12, 30, 0),
        ]
        .map(|(year, month, day, hour, minute, second)| {
            Epoch::from_gregorian_utc(year, month, day, hour, minute, second, 0)
        })
    }

    /// Relative tolerance against satkit. Both sides read the same DE440
    /// integration (anise `.bsp` vs satkit `.440` carry identical Chebyshev
    /// coefficients). Measured worst 2026-07-21: ~2.3e-12 (geocentric Sol,
    /// 1980 - the Earth-split difference dominates); ~40x headroom catches
    /// body-mapping, unit, and time-scale bugs, not precision drift.
    const EPHEMERIS_REL_TOL: f64 = 1e-10;

    /// DE440's Earth/Moon mass ratio, from the ephemeris header (EMRAT).
    /// Used to assemble the true-barycentric Luna reference below.
    const EMRAT: f64 = 81.300_568_221_497_22;

    /// satkit's `barycentric_*` return the RAW stored DE series per body,
    /// and the JPL binary layout stores the Moon geocentrically - so
    /// satkit's "barycentric" Luna is actually Earth-relative. This crate's
    /// contract is the true solar-system barycenter (anise resolves the
    /// 301 -> EMB -> SSB chain), so the Luna reference is assembled here:
    /// Luna_ssb = EMB_ssb + r_geo * EMRAT/(1 + EMRAT).
    fn satkit_barycentric(body: Body, reference: SolarSystem, time: &satkit::Instant) -> DVec3 {
        if body == Body::Luna {
            let emb = vec3(satkit::jplephem::barycentric_pos(SolarSystem::EMB, time).unwrap());
            let geo = vec3(satkit::jplephem::geocentric_pos(SolarSystem::Moon, time).unwrap());
            emb + geo * (EMRAT / (1.0 + EMRAT))
        } else {
            vec3(satkit::jplephem::barycentric_pos(reference, time).unwrap())
        }
    }

    /// State-vector variant of [`satkit_barycentric`], same Luna assembly.
    fn satkit_barycentric_state(
        body: Body,
        reference: SolarSystem,
        time: &satkit::Instant,
    ) -> (DVec3, DVec3) {
        if body == Body::Luna {
            let (emb_p, emb_v) =
                satkit::jplephem::barycentric_state(SolarSystem::EMB, time).unwrap();
            let (geo_p, geo_v) =
                satkit::jplephem::geocentric_state(SolarSystem::Moon, time).unwrap();
            let factor = EMRAT / (1.0 + EMRAT);
            (
                vec3(emb_p) + vec3(geo_p) * factor,
                vec3(emb_v) + vec3(geo_v) * factor,
            )
        } else {
            let (p, v) = satkit::jplephem::barycentric_state(reference, time).unwrap();
            (vec3(p), vec3(v))
        }
    }

    fn vec3(v: satkit::Vector3) -> DVec3 {
        DVec3::new(v[(0, 0)], v[(1, 0)], v[(2, 0)])
    }

    /// Per-component closeness within `rel_tol` scaled by the reference
    /// vector's magnitude (floored at 1 to keep near-zero vectors sane).
    fn assert_vec_close(label: &str, got: DVec3, want: DVec3, rel_tol: f64) {
        let tolerance = rel_tol * want.length().max(1.0);
        let difference = (got - want).abs().max_element();
        assert!(
            difference <= tolerance,
            "{label}: got {got:?}, want {want:?}, |diff| {difference} > {tolerance}"
        );
    }

    #[test]
    fn geocentric_pos_matches_satkit() {
        crate::data::seed_satkit();
        for (epoch_index, &epoch) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} geocentric_pos, epoch {epoch_index}");
                let got = ephemeris::geocentric_pos(crate::data::astro(), body, epoch)
                    .unwrap_or_else(|e| panic!("{label}: {e}"));
                let want =
                    satkit::jplephem::geocentric_pos(reference, &satkit_instant_at_tdb(epoch))
                        .unwrap_or_else(|e| panic!("{label} (satkit): {e}"));
                assert_vec_close(&label, got, vec3(want), EPHEMERIS_REL_TOL);
            }
        }
    }

    #[test]
    fn barycentric_pos_matches_satkit() {
        crate::data::seed_satkit();
        for (epoch_index, &epoch) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} barycentric_pos, epoch {epoch_index}");
                let got = ephemeris::barycentric_pos(crate::data::astro(), body, epoch)
                    .unwrap_or_else(|e| panic!("{label}: {e}"));
                let want = satkit_barycentric(body, reference, &satkit_instant_at_tdb(epoch));
                assert_vec_close(&label, got, want, EPHEMERIS_REL_TOL);
            }
        }
    }

    #[test]
    fn geocentric_state_matches_satkit() {
        crate::data::seed_satkit();
        for (epoch_index, &epoch) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} geocentric_state, epoch {epoch_index}");
                let (got_pos, got_vel) =
                    ephemeris::geocentric_state(crate::data::astro(), body, epoch)
                        .unwrap_or_else(|e| panic!("{label}: {e}"));
                let (want_pos, want_vel) =
                    satkit::jplephem::geocentric_state(reference, &satkit_instant_at_tdb(epoch))
                        .unwrap_or_else(|e| panic!("{label} (satkit): {e}"));
                assert_vec_close(
                    &format!("{label} pos"),
                    got_pos,
                    vec3(want_pos),
                    EPHEMERIS_REL_TOL,
                );
                assert_vec_close(
                    &format!("{label} vel"),
                    got_vel,
                    vec3(want_vel),
                    EPHEMERIS_REL_TOL,
                );
            }
        }
    }

    #[test]
    fn barycentric_state_matches_satkit() {
        crate::data::seed_satkit();
        for (epoch_index, &epoch) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} barycentric_state, epoch {epoch_index}");
                let (got_pos, got_vel) =
                    ephemeris::barycentric_state(crate::data::astro(), body, epoch)
                        .unwrap_or_else(|e| panic!("{label}: {e}"));
                let (want_pos, want_vel) =
                    satkit_barycentric_state(body, reference, &satkit_instant_at_tdb(epoch));
                assert_vec_close(
                    &format!("{label} pos"),
                    got_pos,
                    want_pos,
                    EPHEMERIS_REL_TOL,
                );
                assert_vec_close(
                    &format!("{label} vel"),
                    got_vel,
                    want_vel,
                    EPHEMERIS_REL_TOL,
                );
            }
        }
    }
}

#[cfg(test)]
mod frames {
    use engine_astrodynamics::{Epoch, frames};
    use glam::DQuat;

    use crate::support::satkit_instant;

    /// The GCRF<->ITRF bound: the crate's ITRF93 BPC vs satkit's
    /// IERS-2010 + CelesTrak EOP are different Earth-orientation
    /// realizations; the kernel's own historical accuracy claim is
    /// < 3 urad, and in practice the realizations sit within ~0.2 arcsec
    /// of each other post-1972 (looser before, when EOP data thins out).
    /// Measured worst 2026-07-21: 0.015 arcsec (1980 epoch).
    const GCRF_ITRF_ARCSEC: f64 = 0.05;
    /// Width of the pinned pre-1972 divergence window (see the test doc).
    const GCRF_ITRF_PRE_1972_WINDOW_ARCSEC: f64 = 2.0;
    /// TEME: both sides are equinox-based IAU-76/FK5-class chains.
    /// Measured worst 2026-07-21: 0.22 arcsec (1980 epoch).
    const TEME_ARCSEC: f64 = 0.5;

    fn epochs() -> [Epoch; 4] {
        [
            (1980, 6, 1, 0, 0, 0),
            (2000, 1, 1, 12, 0, 0),
            (2012, 3, 15, 6, 30, 0),
            (2024, 1, 15, 12, 30, 0),
        ]
        .map(|(year, month, day, hour, minute, second)| {
            Epoch::from_gregorian_utc(year, month, day, hour, minute, second, 0)
        })
    }

    fn quat(q: satkit::Quaternion) -> DQuat {
        DQuat::from_xyzw(q.x, q.y, q.z, q.w)
    }

    /// Angle of the relative rotation between two unit quaternions, arcsec.
    fn relative_arcsec(a: DQuat, b: DQuat) -> f64 {
        let rel = a * b.inverse();
        2.0 * rel.w.abs().clamp(0.0, 1.0).acos().to_degrees() * 3600.0
    }

    #[test]
    fn gcrf_itrf_matches_satkit() {
        crate::data::seed_satkit();
        for (index, &epoch) in epochs().iter().enumerate() {
            let angle = relative_arcsec(
                frames::qgcrf2itrf(crate::data::astro(), epoch),
                quat(satkit::frametransform::qgcrf2itrf(&satkit_instant(epoch))),
            );
            assert!(
                angle <= GCRF_ITRF_ARCSEC,
                "qgcrf2itrf epoch {index}: {angle:.4} arcsec"
            );
        }
    }

    /// Before 1972, UTC ran on the rubber-second regime and the two stacks
    /// model it differently: satkit treats pre-1972 TAI-UTC as zero, while
    /// the NAIF BPC on the crate's side carries the true historical time -
    /// on 1965-06-01, TAI-UTC = 3.640130 s + 151 days x 1.296 ms/day
    /// = 3.836 s, which times Terra's 15.041 arcsec/s rotation rate
    /// predicts a 57.70 arcsec divergence. The comparison therefore PINS
    /// that predicted window rather than asserting closeness: the crate
    /// regressing (or satkit gaining a pre-1972 UTC model) both surface as
    /// a moved angle. Post-1972 closeness is the real correctness gate
    /// above.
    #[test]
    fn gcrf_itrf_pre_1972_diverges_by_the_rubber_second_offset() {
        crate::data::seed_satkit();
        let epoch = Epoch::from_gregorian_utc(1965, 6, 1, 0, 0, 0, 0);
        let angle = relative_arcsec(
            frames::qgcrf2itrf(crate::data::astro(), epoch),
            quat(satkit::frametransform::qgcrf2itrf(&satkit_instant(epoch))),
        );
        let predicted = 15.041 * (3.640_130 + 151.0 * 0.001_296);
        assert!(
            (angle - predicted).abs() < GCRF_ITRF_PRE_1972_WINDOW_ARCSEC,
            "qgcrf2itrf 1965: {angle:.4} arcsec, predicted {predicted:.2}"
        );
    }

    #[test]
    fn teme_matches_satkit() {
        crate::data::seed_satkit();
        for (index, &epoch) in epochs().iter().enumerate() {
            let instant = satkit_instant(epoch);
            let to_gcrf = relative_arcsec(
                frames::qteme2gcrf(crate::data::astro(), epoch),
                quat(satkit::frametransform::qteme2gcrf(&instant)),
            );
            assert!(
                to_gcrf <= TEME_ARCSEC,
                "qteme2gcrf epoch {index}: {to_gcrf:.4} arcsec"
            );
            let to_itrf = relative_arcsec(
                frames::qteme2itrf(crate::data::astro(), epoch),
                quat(satkit::frametransform::qteme2itrf(&instant)),
            );
            assert!(
                to_itrf <= TEME_ARCSEC,
                "qteme2itrf epoch {index}: {to_itrf:.4} arcsec"
            );
        }
    }
}

#[cfg(test)]
mod geodetic {
    use engine_astrodynamics::geodetic::geodetic_from_itrf;
    use glam::DVec3;
    use satkit::itrfcoord::ITRFCoord;

    /// Same closed-form problem, same WGS84 constants on both sides - the
    /// bounds are near-machine (1e-9 rad ~ 6 mm on the surface).
    const ANGLE_TOL_RAD: f64 = 1e-9;
    const ALTITUDE_TOL_M: f64 = 1e-3;

    #[test]
    fn matches_satkit_over_grid() {
        crate::data::seed_satkit();
        for lat_deg in [-89.9, -60.0, -30.0, 0.0, 20.0, 45.0, 75.0, 89.9] {
            for lon_deg in [-179.0, -90.0, 0.0, 60.0, 135.0] {
                for alt_m in [-2_000.0_f64, 0.0, 400_000.0] {
                    let (lat, lon) = (f64::to_radians(lat_deg), f64::to_radians(lon_deg));
                    // Any test vector works as long as BOTH sides get the
                    // same one; spherical construction is fine.
                    let radius = 6_378_137.0 + alt_m;
                    let v = DVec3::new(
                        radius * lat.cos() * lon.cos(),
                        radius * lat.cos() * lon.sin(),
                        radius * lat.sin(),
                    );
                    let ours = geodetic_from_itrf(v);
                    let theirs =
                        ITRFCoord::from_vector(&satkit::Vector3::new([[v.x], [v.y], [v.z]]));
                    let (want_lat, want_lon, want_alt) = theirs.to_geodetic_rad();
                    let label = format!("({lat_deg}, {lon_deg}, {alt_m})");
                    assert!(
                        (ours.latitude_rad - want_lat).abs() < ANGLE_TOL_RAD,
                        "lat at {label}"
                    );
                    assert!(
                        (ours.longitude_rad - want_lon).abs() < ANGLE_TOL_RAD,
                        "lon at {label}"
                    );
                    assert!(
                        (ours.altitude_m - want_alt).abs() < ALTITUDE_TOL_M,
                        "alt at {label}: {} vs {want_alt}",
                        ours.altitude_m
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod kepler {
    use engine_astrodynamics::kepler::Kepler;
    use glam::DVec3;

    /// Documented deviation from the plan's 1e-9: the crate's mu comes from
    /// the planetary-constants kernel (DE440 value, 398600.4354 km^3/s^2)
    /// while satkit uses its own constant (WGS84-class 398600.4418) - a
    /// ~1.6e-8 relative gap. At a FIXED perigee state that gap amplifies
    /// into the elements with eccentricity: da/a = (1+e)/(1-e) x dmu/mu
    /// (from 1/a = 2/r - v^2/mu), and dT/T = 1.5 da/a - 0.5 dmu/mu - x199
    /// at e = 0.99. Bounds therefore scale per state (3x headroom over the
    /// mu floor); measured agreement confirms the attribution (e.g.
    /// 1.44e-7 on the e = 0.7 period vs 1.29e-7 predicted).
    const MU_PROVENANCE_REL: f64 = 1.61e-8;

    fn sma_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity) / (1.0 - eccentricity)
    }

    fn period_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.5 * (1.0 + eccentricity) / (1.0 - eccentricity) + 0.5)
    }

    /// Eccentricity itself: de = (1+e) dmu/mu at a fixed perigee state.
    fn ecc_abs_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity)
    }

    /// Terra's GM (m^3/s^2), only for constructing test states.
    const MU_M3_S2: f64 = 3.986004418e14;

    fn satkit_vec(v: DVec3) -> satkit::Vector3 {
        satkit::Vector3::new([[v.x], [v.y], [v.z]])
    }

    /// Perigee states across eccentricities: v_perigee for eccentricity e
    /// at radius r is sqrt(mu (1 + e) / r).
    fn perigee_state(radius_m: f64, eccentricity: f64) -> (DVec3, DVec3) {
        let speed = (MU_M3_S2 * (1.0 + eccentricity) / radius_m).sqrt();
        (DVec3::new(radius_m, 0.0, 0.0), DVec3::new(0.0, speed, 0.0))
    }

    #[test]
    fn elements_match_satkit() {
        crate::data::seed_satkit();
        for eccentricity in [0.0, 0.3, 0.7, 0.99] {
            let (pos, vel) = perigee_state(6_778_000.0, eccentricity);
            let ours = Kepler::from_pv(crate::data::astro(), pos, vel)
                .unwrap_or_else(|e| panic!("crate elements at e = {eccentricity}: {e}"));
            let theirs = satkit::Kepler::from_pv(satkit_vec(pos), satkit_vec(vel))
                .unwrap_or_else(|e| panic!("satkit elements at e = {eccentricity}: {e}"));
            let label = format!("e = {eccentricity}");
            assert!(
                (ours.semi_major_axis_m - theirs.a).abs() / theirs.a < sma_rel_tol(eccentricity),
                "{label}: a {} vs {}",
                ours.semi_major_axis_m,
                theirs.a
            );
            assert!(
                (ours.eccentricity - theirs.eccen).abs() < ecc_abs_tol(eccentricity),
                "{label}: e {} vs {}",
                ours.eccentricity,
                theirs.eccen
            );
            let want_period = theirs.period();
            assert!(
                (ours.period_s - want_period).abs() / want_period < period_rel_tol(eccentricity),
                "{label}: period {} vs {want_period}",
                ours.period_s
            );
        }
    }

    /// Escape states must err on BOTH sides (the crate imposes its own
    /// gate; satkit errs inherently).
    #[test]
    fn escape_state_errs_on_both_sides() {
        crate::data::seed_satkit();
        let (pos, vel) = perigee_state(6_778_000.0, 1.2);
        assert!(Kepler::from_pv(crate::data::astro(), pos, vel).is_err());
        assert!(satkit::Kepler::from_pv(satkit_vec(pos), satkit_vec(vel)).is_err());
    }
}

#[cfg(test)]
mod sgp4 {
    use engine_astrodynamics::{Duration, Epoch, sgp4::sgp4, tle::Tle};

    use crate::support::satkit_instant;

    /// Harness-owned ISS fixture (real checksums - the `sgp4` crate
    /// validates them; satkit ignores them).
    const ISS_TLE: [&str; 3] = [
        "ISS (ZARYA)",
        "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009",
        "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299350",
    ];

    /// Both sides are validated against the Vallado reference C++, so a
    /// miss here means a geopotential-constants (WGS72/84) or time-bridge
    /// bug, not physics.
    const POSITION_TOL_M: f64 = 10.0;
    const VELOCITY_TOL_M_S: f64 = 0.01;

    #[test]
    fn matches_satkit_over_a_two_week_window() {
        crate::data::seed_satkit();
        let ours_tle = Tle::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE");
        let mut theirs_tle =
            satkit::tle::TLE::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE");

        // +-1 week around the element-set epoch at 3-hour steps.
        let t0 = ours_tle.epoch();
        let epochs: Vec<Epoch> = (-56..=56)
            .map(|i| t0 + Duration::from_seconds(3.0 * 3600.0 * f64::from(i)))
            .collect();
        let ours = sgp4(&ours_tle, &epochs).expect("crate sgp4");

        let instants: Vec<satkit::Instant> =
            epochs.iter().map(|&epoch| satkit_instant(epoch)).collect();
        let theirs = satkit::sgp4::sgp4(&mut theirs_tle, &instants).expect("satkit sgp4");

        for (i, state) in ours.iter().enumerate() {
            let want_pos =
                glam::DVec3::new(theirs.pos[(0, i)], theirs.pos[(1, i)], theirs.pos[(2, i)]);
            let want_vel =
                glam::DVec3::new(theirs.vel[(0, i)], theirs.vel[(1, i)], theirs.vel[(2, i)]);
            let pos_diff = (state.pos_teme_m - want_pos).length();
            let vel_diff = (state.vel_teme_m_s - want_vel).length();
            assert!(
                pos_diff <= POSITION_TOL_M,
                "sample {i}: position differs by {pos_diff:.3} m"
            );
            assert!(
                vel_diff <= VELOCITY_TOL_M_S,
                "sample {i}: velocity differs by {vel_diff:.5} m/s"
            );
        }
    }
}

#[cfg(test)]
mod propagation {
    use engine_astrodynamics::propagation::{OrbitState, Settings, propagate};
    use engine_astrodynamics::{Duration, Epoch};
    use glam::DVec3;
    use satkit::orbitprop::{PropSettings, SimpleState};

    use crate::support::satkit_instant;

    /// The full-model bound over a 1-day LEO arc. Honest physical budget,
    /// not a delegation check: EGM2008 vs EGM96 coefficients, our degree-2
    /// solid tides (satkit has none), different integrators at 1e-8
    /// tolerances, satkit's TT-as-TDB third-body positions, and different
    /// Earth-orientation realizations for the harmonic rotation.
    /// Measured 2026-07-21: 1.63 m (1-day full model), 0.72 m (6 h
    /// backward).
    const FULL_MODEL_TOL_M: f64 = 10.0;
    /// Reduced configuration (degree 2, no third bodies, no relativity):
    /// nearly the same physics on both sides - J2-coefficient and
    /// orientation-realization differences remain.
    /// Measured 2026-07-21: 1.09 m.
    const REDUCED_MODEL_TOL_M: f64 = 5.0;

    /// An inclined circular-ish LEO, deliberately not equatorial so J2
    /// secular terms act on the node.
    fn leo_state() -> OrbitState {
        let speed = 7_668.6;
        let inclination = 51.6_f64.to_radians();
        OrbitState {
            pos_gcrf_m: DVec3::new(6_778_000.0, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed * inclination.cos(), speed * inclination.sin()),
        }
    }

    fn satkit_end_state(
        state: &OrbitState,
        begin: Epoch,
        end: Epoch,
        settings: &PropSettings,
    ) -> (DVec3, DVec3) {
        let mut packed = SimpleState::zeros();
        packed[0] = state.pos_gcrf_m.x;
        packed[1] = state.pos_gcrf_m.y;
        packed[2] = state.pos_gcrf_m.z;
        packed[3] = state.vel_gcrf_m_s.x;
        packed[4] = state.vel_gcrf_m_s.y;
        packed[5] = state.vel_gcrf_m_s.z;
        let result = satkit::orbitprop::propagate(
            &packed,
            &satkit_instant(begin),
            &satkit_instant(end),
            settings,
            None,
        )
        .expect("satkit propagation");
        let y = result.state_end;
        (DVec3::new(y[0], y[1], y[2]), DVec3::new(y[3], y[4], y[5]))
    }

    fn satkit_settings(degree: u16, sun_moon: bool, relativity: bool) -> PropSettings {
        PropSettings {
            gravity_degree: degree,
            gravity_order: degree,
            abs_error: 1e-10,
            rel_error: 1e-10,
            use_sun_gravity: sun_moon,
            use_moon_gravity: sun_moon,
            use_relativistic_correction: relativity,
            // Keeps satkit's non-embedded space-weather loader unreachable.
            use_spaceweather: false,
            ..PropSettings::default()
        }
    }

    fn crate_settings(degree: u16, sun_moon: bool, relativity: bool) -> Settings {
        Settings {
            gravity_degree: degree,
            gravity_order: degree,
            abs_error: 1e-10,
            rel_error: 1e-10,
            use_sun_gravity: sun_moon,
            use_moon_gravity: sun_moon,
            use_relativistic_correction: relativity,
            spacecraft: None,
        }
    }

    #[test]
    fn full_model_matches_satkit_over_one_day() {
        crate::data::seed_satkit();
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin + Duration::from_seconds(86_400.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &crate_settings(4, true, true),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, want_vel) =
            satkit_end_state(&state, begin, end, &satkit_settings(4, true, true));
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        let vel_diff = (ours.vel_gcrf_m_s - want_vel).length();
        assert!(
            pos_diff <= FULL_MODEL_TOL_M,
            "1-day full-model position differs by {pos_diff:.2} m"
        );
        assert!(
            vel_diff <= FULL_MODEL_TOL_M / 1000.0,
            "1-day full-model velocity differs by {vel_diff:.4} m/s"
        );
    }

    #[test]
    fn reduced_model_matches_satkit_tightly() {
        crate::data::seed_satkit();
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin + Duration::from_seconds(86_400.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &crate_settings(2, false, false),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, _) = satkit_end_state(&state, begin, end, &satkit_settings(2, false, false));
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        assert!(
            pos_diff <= REDUCED_MODEL_TOL_M,
            "1-day reduced-model position differs by {pos_diff:.2} m"
        );
    }

    /// Backward spans agree with the reference too (satkit integrates
    /// backward natively; the crate re-poses backward arcs as negated
    /// forward problems).
    #[test]
    fn backward_span_matches_satkit() {
        crate::data::seed_satkit();
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin - Duration::from_seconds(6.0 * 3600.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &crate_settings(4, true, true),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, _) = satkit_end_state(&state, begin, end, &satkit_settings(4, true, true));
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        assert!(
            pos_diff <= FULL_MODEL_TOL_M,
            "6-hour backward position differs by {pos_diff:.2} m"
        );
    }
}
