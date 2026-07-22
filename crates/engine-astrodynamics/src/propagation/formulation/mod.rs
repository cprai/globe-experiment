//! Propagation formulations (spec §2/§3): Cowell is the default and the
//! correctness oracle. KS regularization joins as a sibling at refactor
//! P7 - the `Formulation` trait abstraction lands with that second
//! implementation, not before.

pub(crate) mod cowell;
// The KS/switching machinery below is exercised by the spec-§7 battery and
// consumed by the future deep-space scene configurations (spec §3a; owner
// decision §9-Q6) - the LEO facade deliberately never trips it, hence the
// lib-target dead-code allowances.
#[allow(dead_code)]
pub(crate) mod ks;

use glam::DVec3;
use hifitime::{Duration, Epoch};
use nalgebra::SVector;

use super::forces::DynamicsModel;
use super::integrator::{RawArc, SolveConfig, solve_arc_until};
use cowell::{CowellSystem, pack, unpack};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // deep-space scene machinery; exercised by the spec battery
pub(crate) enum FormulationKind {
    Cowell,
    Ks,
}

/// Switch telemetry (spec §3a: log every switch - a high count is a bug
/// signal). Recorded on the returned arc, no logging dependency.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // deep-space scene machinery; exercised by the spec battery
pub(crate) struct SwitchRecord {
    pub t_can: f64,
    pub to: FormulationKind,
}

/// Osculating (eccentricity, periapsis radius) about the canonical
/// central body (mu = 1) - the §3a trigger inputs.
#[allow(dead_code)] // deep-space scene machinery; exercised by the spec battery
pub(crate) fn osculating_e_rp(r: DVec3, v: DVec3) -> (f64, f64) {
    let energy = v.length_squared() / 2.0 - 1.0 / r.length();
    let e_vec = v.cross(r.cross(v)) - r / r.length();
    let e = e_vec.length();
    let semi_major = -1.0 / (2.0 * energy);
    (e, semi_major * (1.0 - e))
}

/// Spec §3a enter/exit trigger functions with their hysteresis bands,
/// positive while the CURRENT formulation should continue. `r_ref_can` is
/// the central body's reference radius in canonical lengths.
#[allow(dead_code)] // deep-space scene machinery; exercised by the spec battery
fn trigger(kind: FormulationKind, r: DVec3, v: DVec3, r_ref_can: f64) -> f64 {
    let (e, rp) = osculating_e_rp(r, v);
    match kind {
        // Enter KS at e > 0.9, or e > 0.6 with periapsis under 3 R.
        FormulationKind::Cowell => (0.9 - e).min((0.6 - e).max(rp / r_ref_can - 3.0)),
        // Leave KS only below BOTH exit bands (e < 0.85; e < 0.55 or
        // periapsis above 3.5 R).
        FormulationKind::Ks => (e - 0.85).max((e - 0.55).min(3.5 - rp / r_ref_can)),
    }
}

/// Automatic-formulation propagation (spec §3a/§5): each segment runs one
/// formulation; trigger crossings are integration events; every switch
/// converts the state, dwells briefly (hysteresis's time guard), and
/// restarts fresh - step-size history is never carried across.
#[allow(dead_code)] // deep-space scene machinery; exercised by the spec battery
pub(crate) fn propagate_switched(
    model: &DynamicsModel,
    anchor: Epoch,
    r0_can: DVec3,
    v0_can: DVec3,
    tf_can: f64,
    config: &SolveConfig,
    r_ref_can: f64,
) -> Result<(RawArc<6>, Vec<SwitchRecord>), String> {
    const MAX_SWITCHES: usize = 64;
    /// Minimum dwell inside a formulation before another switch may fire.
    const DWELL_CAN: f64 = 1.0;

    let mut merged = RawArc {
        t: Vec::new(),
        y: Vec::new(),
        ydot: Vec::new(),
    };
    let mut switches: Vec<SwitchRecord> = Vec::new();
    let (mut t, mut r, mut v) = (0.0_f64, r0_can, v0_can);
    let mut kind = if trigger(FormulationKind::Cowell, r, v, r_ref_can) < 0.0 {
        FormulationKind::Ks
    } else {
        FormulationKind::Cowell
    };
    let extend = |arc: RawArc<6>, merged: &mut RawArc<6>| {
        let skip = usize::from(!merged.t.is_empty());
        merged.t.extend(arc.t.into_iter().skip(skip));
        merged.y.extend(arc.y.into_iter().skip(skip));
        merged.ydot.extend(arc.ydot.into_iter().skip(skip));
    };

    // Epsilon-closed: a KS segment ends at the arrival-event ROOT, which
    // can undershoot tf by the root-finder's tolerance - an exact `<`
    // comparison would spin forever on the residual sliver.
    const TIME_EPS: f64 = 1e-9;
    while t < tf_can - TIME_EPS {
        if switches.len() >= MAX_SWITCHES {
            return Err(format!("formulation chatter: {MAX_SWITCHES} switches"));
        }
        let dwell_end = (t + DWELL_CAN).min(tf_can);
        let (r_end, v_end, crossed) = match kind {
            FormulationKind::Cowell => {
                let system = CowellSystem { model, anchor };
                // Dwell first (no trigger events), then watch the trigger.
                let hop = super::integrator::solve_arc(&system, t, dwell_end, pack(r, v), config)?;
                let mut y_now = *hop.y.last().unwrap();
                extend(hop, &mut merged);
                let mut crossed = false;
                if dwell_end < tf_can {
                    let guard = |_t: f64, y: &SVector<f64, 6>| {
                        let (r, v) = unpack(y);
                        trigger(FormulationKind::Cowell, r, v, r_ref_can)
                    };
                    let none = |_t: f64, _y: &SVector<f64, 6>| 1.0;
                    let outcome = solve_arc_until(
                        &system, dwell_end, tf_can, y_now, config, None, &guard, &none,
                    )?;
                    crossed = outcome.event_t.is_some();
                    y_now = *outcome.arc.y.last().unwrap();
                    extend(outcome.arc, &mut merged);
                }
                let (r_end, v_end) = unpack(&y_now);
                (r_end, v_end, crossed)
            }
            FormulationKind::Ks => {
                let system = ks::KsSystem { model, anchor };
                let mut y_ks = ks::cartesian_to_ks(r, v, t);
                let mut s = 0.0;
                let mut crossed = false;
                // Chunked fictitious-time advance with the arrival event
                // and (after the dwell) the exit trigger.
                for _ in 0..256 {
                    let r_now = ks::ks_to_cartesian(&y_ks).1.length();
                    let t_now = y_ks[9];
                    if t_now >= tf_can - TIME_EPS || crossed {
                        break;
                    }
                    let ds = (1.2 * (tf_can - t_now) / r_now).max(1e-3);
                    let arrival = |_s: f64, y: &SVector<f64, 10>| y[9] - tf_can;
                    let dwell_over = t_now >= dwell_end;
                    let exit_guard = |_s: f64, y: &SVector<f64, 10>| {
                        if !dwell_over {
                            return 1.0;
                        }
                        let (_, r, v) = ks::ks_to_cartesian(y);
                        trigger(FormulationKind::Ks, r, v, r_ref_can)
                    };
                    let outcome = solve_arc_until(
                        &system,
                        s,
                        s + ds,
                        y_ks,
                        config,
                        None,
                        &arrival,
                        &exit_guard,
                    )?;
                    // Convert this chunk's knots to time-domain Cartesian.
                    let mut chunk = RawArc {
                        t: Vec::new(),
                        y: Vec::new(),
                        ydot: Vec::new(),
                    };
                    for knot in &outcome.arc.y {
                        let (t_k, r_k, v_k) = ks::ks_to_cartesian(knot);
                        let epoch = anchor + Duration::from_seconds(model.units.time_to_s(t_k));
                        let a_k = model.acceleration_can(epoch, r_k, v_k)?;
                        chunk.t.push(t_k);
                        chunk.y.push(pack(r_k, v_k));
                        chunk.ydot.push(pack(v_k, a_k));
                    }
                    extend(chunk, &mut merged);
                    y_ks = *outcome.arc.y.last().unwrap();
                    s = *outcome.arc.t.last().unwrap();
                    if outcome.event_t.is_some() {
                        // Arrival and exit share the event plumbing;
                        // discriminate by the trigger's value at the end
                        // state (arrival leaves it positive).
                        let (_, r_end, v_end) = ks::ks_to_cartesian(&y_ks);
                        if trigger(FormulationKind::Ks, r_end, v_end, r_ref_can) <= 1e-9 {
                            crossed = true;
                        }
                    }
                }
                let (_, r_end, v_end) = ks::ks_to_cartesian(&y_ks);
                (r_end, v_end, crossed)
            }
        };
        (r, v) = (r_end, v_end);
        t = *merged.t.last().unwrap();
        if crossed && t < tf_can {
            kind = match kind {
                FormulationKind::Cowell => FormulationKind::Ks,
                FormulationKind::Ks => FormulationKind::Cowell,
            };
            switches.push(SwitchRecord { t_can: t, to: kind });
        }
    }
    Ok((merged, switches))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ephemeris::Body;
    use crate::propagation::bodies::{CentralBody, PointMass};
    use crate::propagation::forces::central::CentralGravity;
    use crate::propagation::forces::third_body::{ThirdBodyGravity, battin_acceleration};
    use crate::propagation::forces::{DynamicsModel, ForceModel};
    use crate::propagation::integrator::solve_arc;
    use crate::propagation::units::CanonicalUnits;

    fn config() -> SolveConfig {
        SolveConfig {
            rtol: 1e-12,
            atol: 1e-12,
            dense_points_per_step: 2,
        }
    }

    fn anchor() -> Epoch {
        Epoch::from_gregorian_utc(2024, 1, 15, 12, 0, 0, 0)
    }

    fn model_about(
        center: Body,
        mu_m3_s2: f64,
        radius_m: f64,
        du_m: f64,
    ) -> DynamicsModel<'static> {
        DynamicsModel {
            data: crate::data::test_data(),
            units: CanonicalUnits::new(mu_m3_s2, du_m),
            center,
            central: CentralGravity {
                field: crate::propagation::forces::harmonics::field_for(
                    crate::data::test_data(),
                    &CentralBody {
                        naif_id: match center {
                            Body::Terra => 399,
                            Body::Sol => 10,
                            Body::Mars => 4,
                            Body::Jupiter => 5,
                            _ => 0,
                        },
                        mu_m3_s2,
                        reference_radius_m: radius_m,
                    },
                    4,
                    4,
                    true,
                ),
            },
            perturbations: Vec::new(),
        }
    }

    /// Spec §7.5: a close flyby three ways - Cowell throughout, KS
    /// throughout, and automatic switching - must agree. (The switched run
    /// being the outlier means conversion or restart logic is wrong.)
    #[test]
    fn flyby_three_ways_agree() {
        let mu = 3.986_004_418e14;
        let model = DynamicsModel {
            data: crate::data::test_data(),
            units: CanonicalUnits::new(mu, 7.0e6),
            center: Body::Terra,
            central: CentralGravity {
                field: Box::new(PointMass {
                    mu_m3_s2: mu,
                    reference_radius_m: 6.378e6,
                }),
            },
            perturbations: vec![Box::new(
                crate::propagation::forces::relativity::Schwarzschild::new(&CanonicalUnits::new(
                    mu, 7.0e6,
                )),
            ) as Box<dyn ForceModel>],
        };
        let (e, rp) = (1.8, 1.4);
        let r0 = DVec3::new(rp, 0.0, 0.0);
        let v0 = DVec3::new(0.0, ((1.0 + e) / rp).sqrt(), 0.0);
        let tf = 3.0;

        let cowell_sys = CowellSystem {
            model: &model,
            anchor: anchor(),
        };
        let cowell = solve_arc(&cowell_sys, 0.0, tf, pack(r0, v0), &config()).unwrap();
        let ks_arc = ks::solve_ks_span(&model, anchor(), r0, v0, 0.0, tf, &config()).unwrap();
        let (switched, switches) =
            propagate_switched(&model, anchor(), r0, v0, tf, &config(), 6.378e6 / 7.0e6).unwrap();
        assert!(switches.is_empty(), "hyperbolic arc stays in KS");

        let (r_c, _v_c) = unpack(cowell.y.last().unwrap());
        for (label, arc) in [("ks", &ks_arc), ("switched", &switched)] {
            let (r_x, v_x) = unpack(arc.y.last().unwrap());
            let dt = tf - arc.t.last().unwrap();
            let r_x = r_x + v_x * dt;
            assert!(
                (r_x - r_c).length() < 1e-7,
                "{label} differs by {:.2e} (endpoint dt = {dt:.3e})",
                (r_x - r_c).length()
            );
        }
    }

    /// Spec §7.6: inside the hysteresis band (enter 0.9 / exit 0.85) a
    /// grazing trajectory must not toggle - two-body eccentricity is
    /// constant, so ANY switch here is chatter.
    #[test]
    fn hysteresis_band_prevents_chatter() {
        let mu = 3.986_004_418e14;
        let model = DynamicsModel {
            data: crate::data::test_data(),
            units: CanonicalUnits::new(mu, 7.0e6),
            center: Body::Terra,
            central: CentralGravity {
                field: Box::new(PointMass {
                    mu_m3_s2: mu,
                    reference_radius_m: 6.378e6,
                }),
            },
            perturbations: Vec::new(),
        };
        // High apoapsis keeps the compound (low-periapsis) trigger clear:
        // only the e-band is in play.
        for e in [0.88, 0.92] {
            let a = 30.0;
            let r0 = DVec3::new(a * (1.0 - e), 0.0, 0.0);
            let v0 = DVec3::new(0.0, ((1.0 + e) / (a * (1.0 - e))).sqrt(), 0.0);
            let tf = 2.0 * std::f64::consts::TAU * a.powf(1.5);
            let (_, switches) =
                propagate_switched(&model, anchor(), r0, v0, tf, &config(), 6.378e6 / 7.0e6)
                    .unwrap();
            assert!(
                switches.is_empty(),
                "e = {e}: {} switches inside the hysteresis band",
                switches.len()
            );
        }
    }

    /// Spec §7.9: the total PHYSICAL acceleration must be identical
    /// whether the state is expressed about Earth (Sun as third body) or
    /// about the Sun (Earth as third body) - a jump means the central /
    /// third-body bookkeeping (direct + indirect terms) is wrong. Pure
    /// algebra over the Battin form, near machine precision.
    #[test]
    fn central_body_switch_preserves_physical_acceleration() {
        let mu_sun = 332_946.05; // in Earth-mu units
        let r = DVec3::new(80.0, 40.0, 20.0); // craft, Earth-centric (~SOI)
        let r_sun = DVec3::new(21_000.0, 9_000.0, 4_000.0); // Sun rel Earth

        // Earth-centric: central Earth + Sun third body; plus Earth's own
        // acceleration toward the Sun.
        let about_earth = -r / r.length().powi(3) + battin_acceleration(mu_sun, r, r_sun);
        let earth_accel = mu_sun * r_sun / r_sun.length().powi(3);

        // Sun-centric: central Sun + Earth third body; plus the Sun's own
        // acceleration toward Earth.
        let rho = r - r_sun; // craft rel Sun
        let rho_earth = -r_sun; // Earth rel Sun
        let about_sun =
            -mu_sun * rho / rho.length().powi(3) + battin_acceleration(1.0, rho, rho_earth);
        let sun_accel = rho_earth / rho_earth.length().powi(3);

        let physical_1 = about_earth + earth_accel;
        let physical_2 = about_sun + sun_accel;
        assert!(
            (physical_1 - physical_2).length() < 1e-12 * physical_1.length(),
            "physical acceleration jumped: {physical_1:?} vs {physical_2:?}"
        );
    }

    /// Spec §7.16 (mandatory): the same close-approach code path at Earth,
    /// Mars, Jupiter, and a small body - only constants change. Earth must
    /// pick up its harmonic field; everyone else falls back to point-mass.
    #[test]
    fn multi_body_genericity() {
        #[allow(clippy::type_complexity)] // (center, mu, radius, third body)
        let bodies: [(Body, f64, f64, Option<(Body, f64)>); 4] = [
            (
                Body::Terra,
                3.986_004_418e14,
                6.378e6,
                Some((Body::Sol, 1.327e20)),
            ),
            (
                Body::Mars,
                4.282_837e13,
                3.396e6,
                Some((Body::Sol, 1.327e20)),
            ),
            (
                Body::Jupiter,
                1.266_865e17,
                6.9911e7,
                Some((Body::Sol, 1.327e20)),
            ),
            (Body::Pluto, 6.3e10, 4.7e5, None), // stand-in small body: no third bodies
        ];
        for (center, mu, radius, third) in bodies {
            let mut model = model_about(center, mu, radius, radius * 1.1);
            if let Some((body, mu3)) = third {
                model.perturbations.push(Box::new(ThirdBodyGravity {
                    body,
                    mu_m3_s2: mu3,
                }));
            }
            let expected_harmonics = center == Body::Terra;
            assert_eq!(
                model.central.field.needs_body_fixed(),
                expected_harmonics,
                "{center:?} registry selection"
            );
            // Hyperbolic close approach through periapsis at 1.5 radii.
            let rp = 1.5 * radius / (radius * 1.1);
            let r0 = DVec3::new(rp, 0.0, 0.0);
            let v0 = DVec3::new(0.0, (2.5_f64 / rp).sqrt(), 0.0); // e = 1.5
            let (arc, _) = propagate_switched(
                &model,
                anchor(),
                r0,
                v0,
                2.0,
                &config(),
                radius / (radius * 1.1),
            )
            .unwrap_or_else(|e| panic!("{center:?} close approach failed: {e}"));
            assert!(arc.t.len() > 2, "{center:?} produced no arc");
        }
    }

    /// Spec §7.10: a real planet propagated against the ephemeris itself,
    /// WITH the Schwarzschild term - the reference dynamics are
    /// relativistic, and the run without it must be visibly worse.
    #[test]
    fn mercury_tracks_the_ephemeris_over_thirty_days() {
        let data = crate::data::test_data();
        let t0 = anchor();
        let days = 30.0;
        let tf_epoch = t0 + Duration::from_seconds(days * 86_400.0);
        let units = CanonicalUnits::new(1.327_124_400_18e20, 1.495_978_707e11);
        let (r0_m, v0_m) =
            crate::ephemeris::relative_state(data, Body::Mercury, Body::Sol, t0).unwrap();
        let (rf_m, _) =
            crate::ephemeris::relative_state(data, Body::Mercury, Body::Sol, tf_epoch).unwrap();
        let tf_can = units.time_to_can(days * 86_400.0);

        let run = |relativity: bool| {
            let mut model = DynamicsModel {
                data,
                units,
                center: Body::Sol,
                central: CentralGravity {
                    field: Box::new(PointMass {
                        // Relative motion about the Sun: the central term
                        // carries mu_Sun + mu_Mercury (the Sun is
                        // accelerated by Mercury too) - omitting the
                        // 1.7e-7 reduced-mass correction costs ~20 km
                        // over these 30 days.
                        mu_m3_s2: units.mu_m3_s2 + 2.203_2e13,
                        reference_radius_m: 6.957e8,
                    }),
                },
                perturbations: vec![
                    Box::new(ThirdBodyGravity {
                        body: Body::Venus,
                        mu_m3_s2: 3.248_585_92e14,
                    }) as Box<dyn ForceModel>,
                    Box::new(ThirdBodyGravity {
                        body: Body::TerraLunaBarycenter,
                        mu_m3_s2: 4.035_032_36e14,
                    }),
                    Box::new(ThirdBodyGravity {
                        body: Body::Jupiter,
                        mu_m3_s2: 1.267_127_64e17,
                    }),
                    Box::new(ThirdBodyGravity {
                        body: Body::Saturn,
                        mu_m3_s2: 3.794_058_5e16,
                    }),
                ],
            };
            if relativity {
                model.perturbations.push(Box::new(
                    crate::propagation::forces::relativity::Schwarzschild::new(&units),
                ));
            }
            let system = CowellSystem {
                model: &model,
                anchor: t0,
            };
            let arc = solve_arc(
                &system,
                0.0,
                tf_can,
                pack(units.length_to_can(r0_m), units.velocity_to_can(v0_m)),
                &config(),
            )
            .unwrap();
            let (r_end, _) = unpack(arc.y.last().unwrap());
            (units.length_to_m(r_end) - rf_m).length()
        };

        let with_relativity = run(true);
        let without = run(false);
        assert!(
            with_relativity < 10_000.0,
            "30-day Mercury error {with_relativity:.0} m"
        );
        assert!(
            without > 1.5 * with_relativity,
            "Schwarzschild must matter: {with_relativity:.0} m vs {without:.0} m without"
        );
    }
}
