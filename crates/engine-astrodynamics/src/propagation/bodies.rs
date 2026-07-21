//! Central-body description and the body-agnostic gravity abstraction
//! (spec §4.0/§4.1): body-specific models are plug-in implementations
//! selected per body, never branches in shared code. Point-mass is the
//! universal default; Earth registers EGM2008 harmonics. (The
//! `AtmosphereModel` registry joins at refactor P6 with drag.)

use glam::DVec3;

/// The body the segment's dynamics are centered on. Constants come from
/// the embedded planetary-constants kernel at config-build time, never
/// hardcoded (spec §4.0).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CentralBody {
    pub naif_id: i32,
    pub mu_m3_s2: f64,
    pub reference_radius_m: f64,
}

/// Central-body gravity (spec §4.1). Implementations return the TOTAL
/// gravitational acceleration (central term included) in m/s^2, evaluated
/// with the model's OWN defining constants - which may legitimately differ
/// from the canonical-unit mu in the low digits (spec §1/§4.1).
pub(crate) trait GravityField {
    /// Acceleration at `r_m` (meters). If [`needs_body_fixed`] is true,
    /// both `r_m` and the result are in the body-fixed frame; otherwise
    /// both are inertial.
    ///
    /// [`needs_body_fixed`]: GravityField::needs_body_fixed
    fn acceleration_m_s2(&self, r_m: DVec3) -> DVec3;

    /// Harmonics: true. Point-mass: false - lets the caller skip two frame
    /// rotations per derivative evaluation on the hot path.
    fn needs_body_fixed(&self) -> bool;

    #[allow(dead_code)] // Consumed by the P5+ SRP/shadow regimes.
    fn mu_m3_s2(&self) -> f64;

    #[allow(dead_code)] // Consumed by the P5+ shadow/drag regimes.
    fn reference_radius_m(&self) -> f64;

    /// Time-dependent coefficient hook (spec §4.1: degree-2 solid tides),
    /// fed the perturbing bodies' BODY-FIXED positions once per derivative
    /// evaluation. Default: static field, no-op.
    fn update_time_dependence(&self, _sun_body_fixed_m: DVec3, _moon_body_fixed_m: DVec3) {}

    /// Whether [`update_time_dependence`] wants to be called at all (saves
    /// the Sun/Moon lookups when the field is static).
    ///
    /// [`update_time_dependence`]: GravityField::update_time_dependence
    fn is_time_dependent(&self) -> bool {
        false
    }
}

/// The universal default: `-mu r / |r|^3` (spec §4.1). Sufficient for
/// every body without a registered harmonic model.
pub(crate) struct PointMass {
    pub mu_m3_s2: f64,
    pub reference_radius_m: f64,
}

impl GravityField for PointMass {
    fn acceleration_m_s2(&self, r_m: DVec3) -> DVec3 {
        let r2 = r_m.length_squared();
        -self.mu_m3_s2 / (r2 * r2.sqrt()) * r_m
    }

    fn needs_body_fixed(&self) -> bool {
        false
    }

    fn mu_m3_s2(&self) -> f64 {
        self.mu_m3_s2
    }

    fn reference_radius_m(&self) -> f64 {
        self.reference_radius_m
    }
}
