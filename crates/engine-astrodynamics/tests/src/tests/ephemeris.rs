//! Crate-vs-reference ephemeris comparisons.

/// The satkit reference side.
mod satkit {
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

/// The astrodyn (JEOD-port) reference side.
mod astrodyn {
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
