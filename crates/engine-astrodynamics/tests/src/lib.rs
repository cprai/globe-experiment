//! Verification harness comparing `engine-astrodynamics` against reference
//! implementations — raw satkit today, other astrodynamics crates later.
//!
//! A test-only lib crate: `cargo test -p engine-astrodynamics-tests` runs
//! the comparisons, while the reference dependencies stay out of every
//! shipped build (nothing depends on this leaf crate). Keep all sources
//! under `src/` with no `main.rs` — the parent crate auto-discovers
//! `tests/*.rs` and `tests/*/main.rs` as its own integration-test targets.
//!
//! Initialization rule: seed satkit's process-wide set-once data stores
//! exclusively through the `Once`-guarded [`engine_astrodynamics::init`].
//! Never call satkit's own `init_*` functions here — the stores accept one
//! seeding per process, and raw satkit queries reading the crate-seeded
//! globals is exactly what the comparison wants (same DE440 bytes on both
//! sides, so only the code paths differ).

#[cfg(test)]
mod ephemeris {
    use engine_astrodynamics::Instant;
    use engine_astrodynamics::ephemeris::{self, Body};
    use glam::DVec3;
    use satkit::SolarSystem;

    /// Test-owned body mapping, deliberately independent of the crate's
    /// private `to_satkit` so the mapping itself is cross-checked.
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
    /// arbitrary modern instants.
    fn epochs() -> [Instant; 4] {
        [
            (1980, 6, 1, 0, 0, 0.0),
            (2000, 1, 1, 12, 0, 0.0),
            (2012, 3, 15, 6, 30, 0.0),
            (2024, 1, 15, 12, 30, 0.0),
        ]
        .map(|(year, month, day, hour, minute, second)| {
            Instant::from_datetime(year, month, day, hour, minute, second)
                .expect("valid test epoch")
        })
    }

    /// Relative tolerance against satkit. The crate currently delegates to
    /// satkit, so agreement is exact; the tolerance keeps the harness valid
    /// once the crate owns its math. Reference crates with independent data
    /// or algorithms will need their own, looser tolerances.
    const SATKIT_REL_TOL: f64 = 1e-12;

    /// Per-component closeness within `rel_tol` scaled by the reference
    /// vector's magnitude (floored at 1 to keep near-zero vectors sane).
    fn assert_vec_close(label: &str, got: DVec3, want: satkit::Vector3, rel_tol: f64) {
        let want = DVec3::new(want[(0, 0)], want[(1, 0)], want[(2, 0)]);
        let tolerance = rel_tol * want.length().max(1.0);
        let difference = (got - want).abs().max_element();
        assert!(
            difference <= tolerance,
            "{label}: got {got:?}, want {want:?}, |diff| {difference} > {tolerance}"
        );
    }

    #[test]
    fn geocentric_pos_matches_satkit() {
        engine_astrodynamics::init();
        for (epoch_index, time) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} geocentric_pos, epoch {epoch_index}");
                let got = ephemeris::geocentric_pos(body, time)
                    .unwrap_or_else(|e| panic!("{label}: {e}"));
                let want = satkit::jplephem::geocentric_pos(reference, time)
                    .unwrap_or_else(|e| panic!("{label} (satkit): {e}"));
                assert_vec_close(&label, got, want, SATKIT_REL_TOL);
            }
        }
    }

    #[test]
    fn barycentric_pos_matches_satkit() {
        engine_astrodynamics::init();
        for (epoch_index, time) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} barycentric_pos, epoch {epoch_index}");
                let got = ephemeris::barycentric_pos(body, time)
                    .unwrap_or_else(|e| panic!("{label}: {e}"));
                let want = satkit::jplephem::barycentric_pos(reference, time)
                    .unwrap_or_else(|e| panic!("{label} (satkit): {e}"));
                assert_vec_close(&label, got, want, SATKIT_REL_TOL);
            }
        }
    }

    #[test]
    fn geocentric_state_matches_satkit() {
        engine_astrodynamics::init();
        for (epoch_index, time) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} geocentric_state, epoch {epoch_index}");
                let (got_pos, got_vel) = ephemeris::geocentric_state(body, time)
                    .unwrap_or_else(|e| panic!("{label}: {e}"));
                let (want_pos, want_vel) = satkit::jplephem::geocentric_state(reference, time)
                    .unwrap_or_else(|e| panic!("{label} (satkit): {e}"));
                assert_vec_close(&format!("{label} pos"), got_pos, want_pos, SATKIT_REL_TOL);
                assert_vec_close(&format!("{label} vel"), got_vel, want_vel, SATKIT_REL_TOL);
            }
        }
    }

    #[test]
    fn barycentric_state_matches_satkit() {
        engine_astrodynamics::init();
        for (epoch_index, time) in epochs().iter().enumerate() {
            for (body, reference) in BODIES {
                let label = format!("{body:?} barycentric_state, epoch {epoch_index}");
                let (got_pos, got_vel) = ephemeris::barycentric_state(body, time)
                    .unwrap_or_else(|e| panic!("{label}: {e}"));
                let (want_pos, want_vel) = satkit::jplephem::barycentric_state(reference, time)
                    .unwrap_or_else(|e| panic!("{label} (satkit): {e}"));
                assert_vec_close(&format!("{label} pos"), got_pos, want_pos, SATKIT_REL_TOL);
                assert_vec_close(&format!("{label} vel"), got_vel, want_vel, SATKIT_REL_TOL);
            }
        }
    }
}
