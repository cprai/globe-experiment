//! Verification harness comparing `engine-astrodynamics` against reference
//! implementations — raw satkit and the astrodyn family (a pure-Rust port
//! of NASA JEOD).
//!
//! A test-only lib crate: `cargo test -p engine-astrodynamics-tests` runs
//! the comparisons, and `cargo bench -p engine-astrodynamics-tests --bench
//! <name>` runs one criterion benchmark target (`src/benches/*.rs` — crate
//! vs reference timing on the same problems; omit `--bench` to run all),
//! while the reference dependencies stay out of every shipped build
//! (nothing depends on this leaf crate). Keep all sources under `src/`
//! with no `main.rs` — the parent crate auto-discovers `tests/*.rs` and
//! `tests/*/main.rs` as its own integration-test targets.
//!
//! The two references overlap deliberately little in provenance: satkit is
//! an independent C-heritage stack with its own DE reader and IERS-2010
//! frames; astrodyn ports JEOD's IAU-76/FK5 RNP, Borkowski geodetic, and
//! JEOD element/propagation pipelines, but shares anise with the crate —
//! so its ephemeris comparison checks wiring (body mapping, units, frame
//! chains) rather than Chebyshev math, and is correspondingly near-machine
//! tight. The `*_astrodyn` modules mirror the satkit modules one domain
//! each; astrodyn has no SGP4, so that domain stays satkit-only.
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

    /// Workspace glam (0.33) -> astrodyn glam (0.30) vector bridge. The
    /// two glam majors carry identical `f64` fields; only the type
    /// identity differs.
    pub fn astrodyn_vec(v: glam::DVec3) -> glam030::DVec3 {
        glam030::DVec3::new(v.x, v.y, v.z)
    }

    /// astrodyn glam (0.30) -> workspace glam (0.33) vector bridge.
    pub fn from_astrodyn_vec(v: glam030::DVec3) -> glam::DVec3 {
        glam::DVec3::new(v.x, v.y, v.z)
    }

    /// astrodyn rotation matrix -> workspace quaternion, column by column
    /// (both glams are column-major, so the axes transfer directly).
    pub fn quat_from_astrodyn_mat(m: glam030::DMat3) -> glam::DQuat {
        glam::DQuat::from_mat3(&glam::DMat3::from_cols(
            from_astrodyn_vec(m.x_axis),
            from_astrodyn_vec(m.y_axis),
            from_astrodyn_vec(m.z_axis),
        ))
    }

    /// The astrodyn RNP's per-epoch time/EOP inputs, bridged from the
    /// harness's satkit EOP table (the crate reads Earth orientation from
    /// its BPC internally and exposes no EOP surface, so the reference
    /// side's inputs come from the other reference's table): accumulated
    /// GMST sidereal seconds since J2000 (via the JEOD Aoki polynomial
    /// over UT1 = UTC + dUT1), TT Julian centuries since J2000, and polar
    /// motion in radians.
    pub fn astrodyn_rnp_inputs(epoch: Epoch) -> (f64, f64, (f64, f64)) {
        crate::data::seed_satkit();
        let eop = satkit::earth_orientation_params::get(&satkit_instant(epoch))
            .expect("EOP available for comparison epoch");
        let arcsec = std::f64::consts::PI / (180.0 * 3600.0);
        let mjd_ut1 = epoch.to_mjd_utc_days() + eop[0] / 86400.0;
        let gmst_seconds =
            astrodyn_time::time_converter_ut1_gmst::ut1_to_gmst_seconds(mjd_ut1 - 51_544.5);
        let tt_centuries = (epoch.to_mjd_tai_days() + 32.184 / 86400.0 - 51_544.5) / 36_525.0;
        (
            gmst_seconds,
            tt_centuries,
            (eop[1] * arcsec, eop[2] * arcsec),
        )
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

#[cfg(test)]
mod ephemeris_astrodyn {
    use astrodyn_ephemeris::EphemerisBody;
    use engine_astrodynamics::Epoch;
    use engine_astrodynamics::ephemeris::{self, Body};
    use glam::DVec3;

    use crate::support::from_astrodyn_vec;

    /// Test-owned body mapping onto the astrodyn reference, independent of
    /// the crate's private NAIF-id table (same rationale as the satkit
    /// mapping above). astrodyn resolves Earth (399) and Moon (301) as
    /// true body centers through anise's chain, so no Luna barycenter
    /// assembly is needed on this side.
    const BODIES: [(Body, EphemerisBody); 11] = [
        (Body::Sol, EphemerisBody::Sun),
        (Body::Mercury, EphemerisBody::Mercury),
        (Body::Venus, EphemerisBody::Venus),
        (
            Body::TerraLunaBarycenter,
            EphemerisBody::EarthMoonBarycenter,
        ),
        (Body::Luna, EphemerisBody::Moon),
        (Body::Mars, EphemerisBody::Mars),
        (Body::Jupiter, EphemerisBody::Jupiter),
        (Body::Saturn, EphemerisBody::Saturn),
        (Body::Uranus, EphemerisBody::Uranus),
        (Body::Neptune, EphemerisBody::Neptune),
        (Body::Pluto, EphemerisBody::Pluto),
    ];

    /// Same epoch spread as the satkit ephemeris module; all inside the
    /// DE440s excerpt span (1849-2150) both sides read.
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

    /// Both sides run anise over byte-identical DE440 Chebyshev data and
    /// take the same `Epoch` value (one shared hifitime), so this bound is
    /// near-machine: it checks body mapping, km->m conversion, and frame
    /// chains, not evaluation math. Measured worst 2026-07-22: 2.2e-16 —
    /// the two stacks agree to the last bit or one ulp.
    const EPHEMERIS_REL_TOL: f64 = 1e-13;

    fn assert_vec_close(label: &str, got: DVec3, want: DVec3, rel_tol: f64) {
        let tolerance = rel_tol * want.length().max(1.0);
        let difference = (got - want).abs().max_element();
        assert!(
            difference <= tolerance,
            "{label}: got {got:?}, want {want:?}, |diff| {difference} > {tolerance}"
        );
    }

    #[test]
    fn geocentric_state_matches_astrodyn() {
        let eph = crate::data::astrodyn_eph();
        for (epoch_index, &epoch) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} geocentric_state, epoch {epoch_index}");
                let (got_pos, got_vel) =
                    ephemeris::geocentric_state(crate::data::astro(), body, epoch)
                        .unwrap_or_else(|e| panic!("{label}: {e}"));
                let (want_pos, want_vel) = eph
                    .get_state_typed_epoch(reference, EphemerisBody::Earth, epoch)
                    .unwrap_or_else(|e| panic!("{label} (astrodyn): {e}"));
                assert_vec_close(
                    &format!("{label} pos"),
                    got_pos,
                    from_astrodyn_vec(want_pos.raw_si()),
                    EPHEMERIS_REL_TOL,
                );
                assert_vec_close(
                    &format!("{label} vel"),
                    got_vel,
                    from_astrodyn_vec(want_vel.raw_si()),
                    EPHEMERIS_REL_TOL,
                );
            }
        }
    }

    /// Also pins the crate's true-solar-system-barycenter contract against
    /// an independently wired anise chain (astrodyn resolves Luna's
    /// 301 -> EMB -> SSB path itself; no EMRAT assembly like the satkit
    /// module needs).
    #[test]
    fn barycentric_state_matches_astrodyn() {
        let eph = crate::data::astrodyn_eph();
        for (epoch_index, &epoch) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} barycentric_state, epoch {epoch_index}");
                let (got_pos, got_vel) =
                    ephemeris::barycentric_state(crate::data::astro(), body, epoch)
                        .unwrap_or_else(|e| panic!("{label}: {e}"));
                let (want_pos, want_vel) = eph
                    .get_state_typed_epoch(reference, EphemerisBody::SolarSystemBarycenter, epoch)
                    .unwrap_or_else(|e| panic!("{label} (astrodyn): {e}"));
                assert_vec_close(
                    &format!("{label} pos"),
                    got_pos,
                    from_astrodyn_vec(want_pos.raw_si()),
                    EPHEMERIS_REL_TOL,
                );
                assert_vec_close(
                    &format!("{label} vel"),
                    got_vel,
                    from_astrodyn_vec(want_vel.raw_si()),
                    EPHEMERIS_REL_TOL,
                );
            }
        }
    }
}

#[cfg(test)]
mod frames_astrodyn {
    use engine_astrodynamics::{Epoch, frames};
    use glam::DQuat;

    use crate::support::{astrodyn_rnp_inputs, quat_from_astrodyn_mat};

    /// The crate's ITRF93 BPC vs astrodyn's JEOD RNP (IAU-76/FK5
    /// precession + 1980 nutation + Aoki GMST + polar motion from the
    /// satkit EOP table). Different Earth-orientation realizations AND a
    /// different UT1 source, plus the ~23 mas J2000<->GCRF frame bias the
    /// JEOD chain does not model. Measured worst 2026-07-22: 0.048 arcsec
    /// (2024 epoch); ~4x headroom.
    const GCRF_ITRF_ARCSEC: f64 = 0.2;

    /// Post-1972 epochs only: the RNP inputs come from the satkit EOP
    /// table, which zeroes the pre-1972 rubber-second regime (see the
    /// pinned 1965 divergence test in the satkit frames module).
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

    fn relative_arcsec(a: DQuat, b: DQuat) -> f64 {
        let rel = a * b.inverse();
        2.0 * rel.w.abs().clamp(0.0, 1.0).acos().to_degrees() * 3600.0
    }

    #[test]
    fn gcrf_itrf_matches_astrodyn_rnp() {
        crate::data::seed_satkit();
        for (index, &epoch) in epochs().iter().enumerate() {
            let (gmst_seconds, tt_centuries, polar) = astrodyn_rnp_inputs(epoch);
            let reference = quat_from_astrodyn_mat(
                astrodyn_frames::rotation_j2000::compute_t_parent_this_with_polar(
                    gmst_seconds,
                    tt_centuries,
                    Some(polar),
                ),
            );
            let angle = relative_arcsec(frames::qgcrf2itrf(crate::data::astro(), epoch), reference);
            assert!(
                angle <= GCRF_ITRF_ARCSEC,
                "qgcrf2itrf vs astrodyn RNP epoch {index}: {angle:.4} arcsec"
            );
        }
    }
}

#[cfg(test)]
mod geodetic_astrodyn {
    use astrodyn_math::GeodeticState;
    use engine_astrodynamics::geodetic::geodetic_from_itrf;
    use glam::DVec3;

    use crate::support::astrodyn_vec;

    /// WGS84 semi-axes handed to astrodyn's planet-agnostic entry point
    /// (the crate's constants are baked into its Vermeille closed form).
    const WGS84_A_M: f64 = 6_378_137.0;
    const WGS84_B_M: f64 = 6_356_752.314_245_179;

    /// Same closed-form problem, same ellipsoid on both sides (Vermeille
    /// vs JEOD's Borkowski iteration) - near-machine bounds, same as the
    /// satkit module.
    const ANGLE_TOL_RAD: f64 = 1e-9;
    const ALTITUDE_TOL_M: f64 = 1e-3;

    #[test]
    fn matches_astrodyn_over_grid() {
        for lat_deg in [-89.9, -60.0, -30.0, 0.0, 20.0, 45.0, 75.0, 89.9] {
            for lon_deg in [-179.0, -90.0, 0.0, 60.0, 135.0] {
                for alt_m in [-2_000.0_f64, 0.0, 400_000.0] {
                    let (lat, lon) = (f64::to_radians(lat_deg), f64::to_radians(lon_deg));
                    let radius = WGS84_A_M + alt_m;
                    let v = DVec3::new(
                        radius * lat.cos() * lon.cos(),
                        radius * lat.cos() * lon.sin(),
                        radius * lat.sin(),
                    );
                    let ours = geodetic_from_itrf(v);
                    let theirs =
                        GeodeticState::from_planet_fixed(astrodyn_vec(v), WGS84_A_M, WGS84_B_M);
                    let label = format!("({lat_deg}, {lon_deg}, {alt_m})");
                    assert!(
                        (ours.latitude_rad - theirs.latitude).abs() < ANGLE_TOL_RAD,
                        "lat at {label}: {} vs {}",
                        ours.latitude_rad,
                        theirs.latitude
                    );
                    assert!(
                        (ours.longitude_rad - theirs.longitude).abs() < ANGLE_TOL_RAD,
                        "lon at {label}: {} vs {}",
                        ours.longitude_rad,
                        theirs.longitude
                    );
                    assert!(
                        (ours.altitude_m - theirs.altitude).abs() < ALTITUDE_TOL_M,
                        "alt at {label}: {} vs {}",
                        ours.altitude_m,
                        theirs.altitude
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod kepler_astrodyn {
    use astrodyn_math::OrbitalElements;
    use astrodyn_quantities::frame::{Earth, PlanetInertial};
    use astrodyn_quantities::prelude::*;
    use engine_astrodynamics::kepler::Kepler;
    use glam::DVec3;

    use crate::support::astrodyn_vec;

    /// Same mu-provenance reasoning as the satkit kepler module: astrodyn
    /// is handed the satkit-class mu below, so the crate's pca-derived mu
    /// differs by the same ~1.6e-8 relative gap, amplified into the
    /// elements at a fixed perigee state (see that module's derivation).
    const MU_PROVENANCE_REL: f64 = 1.61e-8;

    fn sma_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity) / (1.0 - eccentricity)
    }

    fn period_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.5 * (1.0 + eccentricity) / (1.0 - eccentricity) + 0.5)
    }

    fn ecc_abs_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity)
    }

    /// Terra's GM (m^3/s^2), for test states AND as astrodyn's handed mu.
    const MU_M3_S2: f64 = 3.986004418e14;

    fn perigee_state(radius_m: f64, eccentricity: f64) -> (DVec3, DVec3) {
        let speed = (MU_M3_S2 * (1.0 + eccentricity) / radius_m).sqrt();
        (DVec3::new(radius_m, 0.0, 0.0), DVec3::new(0.0, speed, 0.0))
    }

    fn astrodyn_elements(pos: DVec3, vel: DVec3) -> OrbitalElements<Earth> {
        OrbitalElements::from_cartesian_typed(
            MU_M3_S2.m3_per_s2_for::<Earth>(),
            astrodyn_vec(pos).m_at::<PlanetInertial<Earth>>(),
            astrodyn_vec(vel).m_per_s_at::<PlanetInertial<Earth>>(),
        )
        .expect("astrodyn elements")
    }

    /// e stops at 0.95 (not the satkit module's 0.99): JEOD declares
    /// |e - 1| <= 0.01 parabolic and nulls the semi-major axis — see the
    /// contract-difference test below.
    #[test]
    fn elements_match_astrodyn() {
        for eccentricity in [0.0, 0.3, 0.7, 0.95] {
            let (pos, vel) = perigee_state(6_778_000.0, eccentricity);
            let ours = Kepler::from_pv(crate::data::astro(), pos, vel)
                .unwrap_or_else(|e| panic!("crate elements at e = {eccentricity}: {e}"));
            let theirs = astrodyn_elements(pos, vel);
            let label = format!("e = {eccentricity}");
            assert!(
                (ours.semi_major_axis_m - theirs.semi_major_axis).abs() / theirs.semi_major_axis
                    < sma_rel_tol(eccentricity),
                "{label}: a {} vs {}",
                ours.semi_major_axis_m,
                theirs.semi_major_axis
            );
            assert!(
                (ours.eccentricity - theirs.e_mag).abs() < ecc_abs_tol(eccentricity),
                "{label}: e {} vs {}",
                ours.eccentricity,
                theirs.e_mag
            );
            let want_period = std::f64::consts::TAU / theirs.mean_motion;
            assert!(
                (ours.period_s - want_period).abs() / want_period < period_rel_tol(eccentricity),
                "{label}: period {} vs {want_period}",
                ours.period_s
            );
        }
    }

    /// Contract differences, pinned: the crate refuses escape states (its
    /// e >= 1 gate) while JEOD's element set is defined for hyperbolic
    /// orbits (negative semi-major axis) - astrodyn must succeed and
    /// report the same eccentricity regime, not err. And inside JEOD's
    /// parabolic band (|e - 1| <= its ORBIT_SWITCH_TOL of 1e-2) astrodyn
    /// nulls the semi-major axis to 0.0 where the crate still returns the
    /// finite ellipse - which is why the element comparison above stops
    /// at e = 0.95.
    #[test]
    fn escape_and_parabolic_band_contract_differences() {
        let (pos, vel) = perigee_state(6_778_000.0, 1.2);
        assert!(Kepler::from_pv(crate::data::astro(), pos, vel).is_err());
        let theirs = astrodyn_elements(pos, vel);
        assert!(
            theirs.e_mag > 1.0 && theirs.semi_major_axis < 0.0,
            "astrodyn hyperbolic elements: e {} a {}",
            theirs.e_mag,
            theirs.semi_major_axis
        );

        let (pos, vel) = perigee_state(6_778_000.0, 0.99);
        assert!(Kepler::from_pv(crate::data::astro(), pos, vel).is_ok());
        let theirs = astrodyn_elements(pos, vel);
        assert!(
            theirs.semi_major_axis == 0.0 && theirs.semiparam > 0.0,
            "astrodyn parabolic-band elements: a {} p {}",
            theirs.semi_major_axis,
            theirs.semiparam
        );
    }
}

#[cfg(test)]
mod propagation_astrodyn {
    use astrodyn::{
        FrameUid, GravityControl, GravityGradient, SimulationBuilder, TranslationalStateTyped,
        VehicleBuilder,
    };
    use astrodyn_quantities::frame::{Earth, PlanetInertial, RootInertial};
    use astrodyn_quantities::prelude::*;
    use engine_astrodynamics::kepler::Kepler;
    use engine_astrodynamics::propagation::{OrbitState, Settings, propagate};
    use engine_astrodynamics::{Duration, Epoch};
    use glam::DVec3;

    use crate::support::{astrodyn_vec, from_astrodyn_vec};

    /// astrodyn's fixed RK4 step. At 5 s (~1100 steps/orbit) the RK4
    /// truncation is far below the comparison bound, so the bound
    /// measures model agreement, not integrator gap.
    const ASTRODYN_DT_S: f64 = 5.0;

    /// One day, mu-matched two-body problem on both sides (the crate at
    /// point-mass degradation, astrodyn at point-mass gravity with its
    /// source mu overwritten by the crate's own pca-derived value) - the
    /// residual is pure integrator truncation. Measured 2026-07-22:
    /// 0.0074 m position, 8e-6 m/s velocity over the day (~16 orbits).
    const TWO_BODY_TOL_M: f64 = 0.1;

    /// Same inclined LEO as the satkit propagation module.
    fn leo_state() -> OrbitState {
        let speed = 7_668.6;
        let inclination = 51.6_f64.to_radians();
        OrbitState {
            pos_gcrf_m: DVec3::new(6_778_000.0, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed * inclination.cos(), speed * inclination.sin()),
        }
    }

    /// The crate's central-body mu (m^3/s^2), recovered from its own
    /// osculating elements (period = TAU sqrt(a^3/mu)) since the pca
    /// value is not exposed directly. Machine-precision round trip.
    fn crate_mu_m3_s2(state: &OrbitState) -> f64 {
        let kepler = Kepler::from_pv(crate::data::astro(), state.pos_gcrf_m, state.vel_gcrf_m_s)
            .expect("elliptic LEO state");
        let a = kepler.semi_major_axis_m;
        std::f64::consts::TAU.powi(2) * a.powi(3) / kepler.period_s.powi(2)
    }

    /// Point-mass-only crate settings: degree < 2 degrades EGM2008 to
    /// point-mass, sun/moon/relativity off, no spacecraft - a pure
    /// two-body problem.
    fn two_body_settings() -> Settings {
        Settings {
            gravity_degree: 0,
            gravity_order: 0,
            abs_error: 1e-10,
            rel_error: 1e-10,
            use_sun_gravity: false,
            use_moon_gravity: false,
            use_relativistic_correction: false,
            spacecraft: None,
        }
    }

    /// One-day astrodyn two-body end state via the JEOD pipeline runner
    /// (point-mass Earth at origin, RK4, epoch irrelevant to the physics).
    pub(crate) fn astrodyn_two_body_end(
        state: &OrbitState,
        mu_m3_s2: f64,
        span_s: f64,
    ) -> (DVec3, DVec3) {
        let mut entry = astrodyn::recipes::earth::point_mass();
        entry.source.mu = mu_m3_s2;
        let mut builder = SimulationBuilder::new(astrodyn::recipes::epoch::j2000(), ASTRODYN_DT_S);
        builder.add_source("Earth", entry);
        let vehicle = VehicleBuilder::new()
            .vehicle_named("probe")
            .with_translational(TranslationalStateTyped::<RootInertial> {
                position: astrodyn_vec(state.pos_gcrf_m).m_at::<RootInertial>(),
                velocity: astrodyn_vec(state.vel_gcrf_m_s).m_per_s_at::<RootInertial>(),
            })
            .three_dof_point_mass(1.0.kg())
            .rk4()
            .gravity(GravityControl::new_spherical(
                FrameUid::of::<PlanetInertial<Earth>>(),
                GravityGradient::Skip,
            ))
            .build();
        builder.add_body(vehicle);
        let mut sim =
            astrodyn_runner::Simulation::from_builder(builder).expect("valid astrodyn simulation");
        sim.step_until(span_s).expect("astrodyn propagation");
        // With the single source at the origin the integration frame and
        // the root inertial frame coincide, so the integrated state IS
        // the inertial state (no integ-origin shift to apply).
        let out = sim.body(0);
        (
            from_astrodyn_vec(out.trans.position.raw_si()),
            from_astrodyn_vec(out.trans.velocity.raw_si()),
        )
    }

    #[test]
    fn two_body_day_matches_astrodyn() {
        let state = leo_state();
        let begin = Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0);
        let end = begin + Duration::from_seconds(86_400.0);
        let ours = propagate(
            crate::data::astro(),
            &state,
            begin,
            end,
            &two_body_settings(),
        )
        .expect("crate propagation")
        .state_end();
        let (want_pos, want_vel) = astrodyn_two_body_end(&state, crate_mu_m3_s2(&state), 86_400.0);
        let pos_diff = (ours.pos_gcrf_m - want_pos).length();
        let vel_diff = (ours.vel_gcrf_m_s - want_vel).length();
        assert!(
            pos_diff <= TWO_BODY_TOL_M,
            "1-day two-body position differs by {pos_diff:.3} m"
        );
        assert!(
            vel_diff <= TWO_BODY_TOL_M / 1000.0,
            "1-day two-body velocity differs by {vel_diff:.5} m/s"
        );
    }
}
