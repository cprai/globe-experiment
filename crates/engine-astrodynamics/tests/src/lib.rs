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
//! tight. The comparisons live one file per domain
//! (`src/tests/<domain>.rs`, the same split as the bench targets), with
//! one nested module per reference implementation (`satkit`, `astrodyn`,
//! and future comparison crates as siblings). astrodyn has no SGP4, so
//! that domain stays satkit-only.
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
mod tests;
