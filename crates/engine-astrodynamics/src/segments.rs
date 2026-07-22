//! Load-time pre-resolved kernel segments - the performance layer behind
//! the ephemeris and Earth-rotation queries.
//!
//! anise's `Almanac::translate`/`rotate` re-resolve the frame tree on every
//! call (root sweep over every loaded summary, two path walks, a zero-copy
//! re-derivation of each segment view from raw DAF bytes, and repeated
//! `Epoch` <-> ET-seconds conversions). All of that is invariant for our
//! fixed embedded kernel set, so [`AstroData`]'s load pre-resolves it ONCE:
//! each DAF segment is flattened into its record grid (coefficient slice
//! over the `'static` embedded bytes + the segment constants already
//! converted), and a query is reduced to one epoch->ET conversion, an
//! integer-comparison segment pick, and anise's own [`chebyshev_eval`] -
//! the same coefficients through the same interpolation code, so results
//! match the almanac path to machine precision (proven by the comparison
//! tests below). No caching happens at query time; everything
//! epoch-independent is precomputed, everything epoch-dependent is
//! recomputed exactly, per call.
//!
//! [`AstroData`]: crate::data::AstroData

use std::collections::HashMap;

use anise::constants::orientations::{ECLIPJ2000, J2000, J2000_TO_ECLIPJ2000_ANGLE_RAD};
use anise::math::interpolation::chebyshev_eval;
use anise::math::rotation::{DCM, r1, r1_dot, r3, r3_dot};
use anise::math::{Matrix3, Vector3};
use anise::naif::daf::datatypes::{Type2ChebyshevRecord, Type2ChebyshevSet};
use anise::naif::daf::{DafDataType, NAIFDataRecord, NAIFDataSet, NAIFSummaryRecord};
use anise::prelude::{BPC, SPK};
use hifitime::{Epoch, Unit};
use zerocopy::Ref;

/// The solar-system barycenter, root of the DE440 ephemeris tree.
const SSB: i32 = 0;

/// One pre-resolved Type-2 Chebyshev segment, flattened at load: the
/// containment bounds carry anise's ±100 ns summary slack pre-applied
/// (`DAF::summary_from_id_at_epoch`) and are stored as TAI duration parts,
/// because `Epoch` comparison itself calls `to_tai_duration()`, a
/// trig-bearing TDB conversion for the summaries' ET-scale epochs, on
/// EVERY compare; converting once at load (and the query epoch once per
/// call) makes selection pure integer comparison with identical ordering.
/// The record-grid constants anise re-derives from the DAF footer per
/// evaluation are likewise stored converted.
struct Segment {
    min_tai: (i16, u64),
    max_tai: (i16, u64),
    init_et_s: f64,
    radius_s: f64,
    rsize: usize,
    num_records: usize,
    degree: usize,
    record_data: &'static [f64],
}

impl Segment {
    /// Flattens one DAF segment - the same `(start_index - 1) * 8 ..
    /// end_index * 8` slice and zero-copy `[u8] -> [f64]` cast as anise's
    /// `DAF::nth_data`, done once here instead of per query. The embeds
    /// are 8-aligned by construction (`Align8` in data.rs), so the cast
    /// cannot fail on alignment.
    fn new(bytes: &'static [u8], summary: &impl NAIFSummaryRecord) -> Self {
        assert_eq!(
            summary.data_type().expect("segment data type"),
            DafDataType::Type2ChebyshevTriplet,
            "pre-resolved kernels are expected to hold Type 2 Chebyshev segments only"
        );
        let start = (summary.start_index() - 1) * size_of::<f64>();
        let end = summary.end_index() * size_of::<f64>();
        let floats: &'static [f64] = Ref::into_ref(
            Ref::<&'static [u8], [f64]>::from_bytes(&bytes[start..end])
                .expect("8-aligned embedded kernel bytes"),
        );
        let set = Type2ChebyshevSet::from_f64_slice(floats).expect("parse embedded segment");
        Self {
            min_tai: (summary.start_epoch() - Unit::Nanosecond * 100)
                .to_tai_duration()
                .to_parts(),
            max_tai: (summary.end_epoch() + Unit::Nanosecond * 100)
                .to_tai_duration()
                .to_parts(),
            init_et_s: set.init_epoch.to_et_seconds(),
            radius_s: set.interval_length.to_seconds() / 2.0,
            rsize: set.rsize,
            num_records: set.num_records,
            degree: set.degree(),
            record_data: set.record_data,
        }
    }

    /// `tai` is the query epoch's `to_tai_duration().to_parts()` - the
    /// exact quantity hifitime's `Epoch` ordering compares.
    fn contains(&self, tai: (i16, u64)) -> bool {
        self.min_tai <= tai && tai <= self.max_tai
    }

    /// (position, rate) at the epoch, whose ET seconds the caller
    /// converted ONCE per query - anise's `Type2ChebyshevSet::evaluate`
    /// record selection and [`chebyshev_eval`] over the pre-flattened
    /// grid (`epoch` itself only labels interpolation errors).
    fn eval(&self, epoch: Epoch, et_s: f64) -> Result<(Vector3, Vector3), String> {
        let spline_idx =
            (((et_s - self.init_et_s) / (2.0 * self.radius_s)) as usize + 1).min(self.num_records);
        let record = Type2ChebyshevRecord::from_slice_f64(
            &self.record_data[(spline_idx - 1) * self.rsize..spline_idx * self.rsize],
        );
        let normalized_time = (et_s - record.midpoint_et_s) / self.radius_s;

        let mut state = Vector3::zeros();
        let mut rate = Vector3::zeros();
        for (index, coeffs) in [record.x_coeffs, record.y_coeffs, record.z_coeffs]
            .into_iter()
            .enumerate()
        {
            let (value, derivative) =
                chebyshev_eval(normalized_time, coeffs, self.radius_s, epoch, self.degree)
                    .map_err(|error| format!("Chebyshev evaluation at {epoch}: {error}"))?;
            state[index] = value;
            rate[index] = derivative;
        }
        Ok((state, rate))
    }
}

/// Every ephemeris segment of the embedded SPKs, grouped per target body
/// and pre-ordered in anise's search order, plus the (static) tree edges.
pub(crate) struct EphemerisTree {
    nodes: HashMap<i32, EphemerisNode>,
}

struct EphemerisNode {
    /// The body this node's states are expressed against (its DE440
    /// parent) - constant across the node's segments, asserted at load.
    center: i32,
    /// Search order mirrors `Almanac::spk_summary_at_epoch`: files in
    /// reverse load order, and within a file descending start epoch (the
    /// almanac's partition-point pick for non-overlapping segments).
    segments: Vec<Segment>,
}

impl EphemerisTree {
    /// Pre-resolves every segment of `files`, given IN SEARCH ORDER
    /// (reverse almanac load order - the overlap-serving excerpt first).
    pub(crate) fn new(files: &[(&SPK, &'static [u8])]) -> Self {
        let mut nodes: HashMap<i32, EphemerisNode> = HashMap::new();
        for (spk, bytes) in files {
            let mut per_file: HashMap<i32, Vec<(Epoch, Segment, i32)>> = HashMap::new();
            for block in spk.iter_summary_blocks().flatten() {
                for summary in block {
                    if summary.is_empty() {
                        continue;
                    }
                    per_file.entry(summary.target_id).or_default().push((
                        summary.start_epoch(),
                        Segment::new(bytes, summary),
                        summary.center_id,
                    ));
                }
            }
            for (target, mut segments) in per_file {
                segments.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
                let center = segments[0].2;
                let node = nodes.entry(target).or_insert_with(|| EphemerisNode {
                    center,
                    segments: Vec::new(),
                });
                for (_, segment, segment_center) in segments {
                    assert_eq!(
                        node.center, segment_center,
                        "body {target} changes ephemeris center across segments"
                    );
                    node.segments.push(segment);
                }
            }
        }
        Self { nodes }
    }

    /// (position km, velocity km/s) of `target` relative to `observer`,
    /// J2000 orientation - the same leaf-to-root leg accumulation as
    /// `Almanac::translate`, over the pre-resolved segments.
    pub(crate) fn state_km(
        &self,
        target: i32,
        observer: i32,
        epoch: Epoch,
    ) -> Result<(Vector3, Vector3), String> {
        if target == observer {
            return Ok((Vector3::zeros(), Vector3::zeros()));
        }
        let et_s = epoch.to_et_seconds();
        let tai = epoch.to_tai_duration().to_parts();
        let path_target = self.path_to_root(target)?;
        let path_observer = self.path_to_root(observer)?;
        let common = *path_target
            .iter()
            .find(|id| path_observer.contains(id))
            .expect("both paths reach the SSB root");

        let mut position = Vector3::zeros();
        let mut velocity = Vector3::zeros();
        for &id in path_target.iter().take_while(|&&id| id != common) {
            let (leg_position, leg_velocity) = self.leg_km(id, epoch, tai, et_s)?;
            position += leg_position;
            velocity += leg_velocity;
        }
        for &id in path_observer.iter().take_while(|&&id| id != common) {
            let (leg_position, leg_velocity) = self.leg_km(id, epoch, tai, et_s)?;
            position -= leg_position;
            velocity -= leg_velocity;
        }
        Ok((position, velocity))
    }

    /// The chain of body ids from `id` up to (and including) the SSB.
    fn path_to_root(&self, mut id: i32) -> Result<Vec<i32>, String> {
        let mut path = Vec::with_capacity(4);
        while id != SSB {
            path.push(id);
            id = self
                .nodes
                .get(&id)
                .ok_or_else(|| format!("no ephemeris data for NAIF id {id}"))?
                .center;
        }
        path.push(SSB);
        Ok(path)
    }

    /// One tree edge: the state of `id` relative to its DE440 parent.
    fn leg_km(
        &self,
        id: i32,
        epoch: Epoch,
        tai: (i16, u64),
        et_s: f64,
    ) -> Result<(Vector3, Vector3), String> {
        let node = &self.nodes[&id];
        let segment = node
            .segments
            .iter()
            .find(|segment| segment.contains(tai))
            .ok_or_else(|| format!("no ephemeris segment covers {epoch} for NAIF id {id}"))?;
        segment.eval(epoch, et_s)
    }
}

/// The embedded Earth PCK's ITRF93 segments, pre-resolved: the J2000 ->
/// ITRF93 rotation without anise's per-call orientation-graph walk.
pub(crate) struct EarthRotation {
    itrf93_id: i32,
    /// Descending start epoch (same pick as the almanac path).
    segments: Vec<Segment>,
    /// The constant leg from J2000 to the segments' stored parent frame
    /// (the combined Earth PCK expresses ITRF93 against ECLIPJ2000, so
    /// this is anise's embedded obliquity rotation; identity when a
    /// replacement kernel stores J2000 directly). Constant, so its time
    /// derivative is zero and the chain derivative is one product.
    j2000_to_parent: Matrix3,
}

impl EarthRotation {
    /// Pre-resolves the BPC's ITRF93 segments. The fast path composes only
    /// the one constant J2000 leg anise's graph would, so every segment
    /// must be expressed against J2000 or ECLIPJ2000 - asserted here (a
    /// replacement kernel breaking that must fail the build, not silently
    /// rotate through a missing leg).
    pub(crate) fn new(bpc: &BPC, bytes: &'static [u8], itrf93_id: i32) -> Self {
        let mut segments = Vec::new();
        let mut parent = None;
        for block in bpc.iter_summary_blocks().flatten() {
            for summary in block {
                if summary.is_empty() || summary.frame_id != itrf93_id {
                    continue;
                }
                assert!(
                    parent.is_none_or(|id| id == summary.inertial_frame_id),
                    "Earth PCK mixes parent frames across ITRF93 segments"
                );
                parent = Some(summary.inertial_frame_id);
                segments.push((summary.start_epoch(), Segment::new(bytes, summary)));
            }
        }
        assert!(!segments.is_empty(), "Earth PCK holds no ITRF93 segments");
        segments.sort_by_key(|(start, _)| std::cmp::Reverse(*start));
        let j2000_to_parent = match parent.expect("segments imply a parent") {
            J2000 => Matrix3::identity(),
            ECLIPJ2000 => r1(J2000_TO_ECLIPJ2000_ANGLE_RAD),
            other => panic!("Earth PCK segments expressed against unsupported frame {other}"),
        };
        Self {
            itrf93_id,
            segments: segments.into_iter().map(|(_, segment)| segment).collect(),
            j2000_to_parent,
        }
    }

    /// The J2000 -> ITRF93 DCM with its time derivative - the identical
    /// Euler-angle evaluation and matrix assembly as anise's
    /// `rotation_to_parent`, over the pre-resolved segment, composed with
    /// the constant J2000 -> parent leg.
    pub(crate) fn dcm_j2000_to_itrf93(&self, epoch: Epoch) -> Result<DCM, String> {
        let tai = epoch.to_tai_duration().to_parts();
        let segment = self
            .segments
            .iter()
            .find(|segment| segment.contains(tai))
            .ok_or_else(|| format!("no Earth PCK segment covers {epoch}"))?;
        let (ra_dec_w, d_ra_dec_w) = segment
            .eval(epoch, epoch.to_et_seconds())
            .map_err(|error| format!("Earth PCK evaluation: {error}"))?;

        let ra_rad = ra_dec_w[0];
        let dec_rad = ra_dec_w[1];
        let twist_rad = ra_dec_w[2];
        let ra_dot_rad = d_ra_dec_w[0];
        let dec_dot_rad = d_ra_dec_w[1];
        let twist_dot_rad = d_ra_dec_w[2];

        // Each constructor computes a sincos; anise re-invokes them per
        // product term - hoisted here (same values, same left-to-right
        // association as anise's expression).
        let r3_twist = r3(twist_rad);
        let r1_dec = r1(dec_rad);
        let r3_ra = r3(ra_rad);
        let parent_to_itrf = r3_twist * r1_dec * r3_ra;
        let parent_to_itrf_dt = twist_dot_rad * r3_dot(twist_rad) * r1_dec * r3_ra
            + dec_dot_rad * r3_twist * r1_dot(dec_rad) * r3_ra
            + ra_dot_rad * r3_twist * r1_dec * r3_dot(ra_rad);
        Ok(DCM {
            rot_mat: parent_to_itrf * self.j2000_to_parent,
            rot_mat_dt: Some(parent_to_itrf_dt * self.j2000_to_parent),
            from: J2000,
            to: self.itrf93_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use anise::constants::frames::{EARTH_ITRF93, EARTH_J2000, SSB_J2000};
    use hifitime::Epoch;

    use crate::data::test_data;
    use crate::ephemeris::Body;

    const ALL: [Body; 12] = [
        Body::Sol,
        Body::Terra,
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

    fn epochs() -> [Epoch; 3] {
        [
            Epoch::from_gregorian_utc(1965, 3, 2, 6, 0, 0, 0),
            Epoch::from_gregorian_utc(2020, 1, 1, 0, 0, 0, 0),
            Epoch::from_gregorian_utc(2100, 7, 4, 12, 0, 0, 0),
        ]
    }

    /// The pre-resolved tree must reproduce `Almanac::translate` (same
    /// coefficients through the same evaluation code) - any drift means
    /// the segment slicing, selection, or leg accumulation diverged.
    #[test]
    fn ephemeris_matches_almanac() {
        let data = test_data();
        for epoch in epochs() {
            for body in ALL {
                for observer in [EARTH_J2000, SSB_J2000] {
                    let reference = data
                        .almanac
                        .translate(body.frame(), observer, epoch, None)
                        .expect("almanac translate");
                    let (pos, vel) = data
                        .ephemeris_segments
                        .state_km(body.naif_id(), observer.ephemeris_id, epoch)
                        .expect("fast-path state");
                    let dp = (pos - reference.radius_km).norm();
                    let dv = (vel - reference.velocity_km_s).norm();
                    assert!(
                        dp <= 1e-9 * reference.radius_km.norm().max(1.0),
                        "{body:?} wrt {} at {epoch}: {dp} km off",
                        observer.ephemeris_id
                    );
                    assert!(
                        dv <= 1e-9 * reference.velocity_km_s.norm().max(1e-6),
                        "{body:?} wrt {} at {epoch}: {dv} km/s off",
                        observer.ephemeris_id
                    );
                }
            }
        }
    }

    /// Same for the Earth rotation: the pre-resolved BPC segment must
    /// reproduce `Almanac::rotate`'s DCM and derivative.
    #[test]
    fn earth_rotation_matches_almanac() {
        let data = test_data();
        for epoch in epochs() {
            let reference = data
                .almanac
                .rotate(EARTH_J2000, EARTH_ITRF93, epoch)
                .expect("almanac rotate");
            let dcm = data
                .earth_rotation
                .dcm_j2000_to_itrf93(epoch)
                .expect("fast-path rotation");
            assert!(
                (dcm.rot_mat - reference.rot_mat).norm() < 1e-14,
                "rotation diverged at {epoch}"
            );
            let (dt, reference_dt) = (
                dcm.rot_mat_dt.expect("fast-path derivative"),
                reference.rot_mat_dt.expect("almanac derivative"),
            );
            assert!(
                (dt - reference_dt).norm() < 1e-14,
                "rotation derivative diverged at {epoch}"
            );
        }
    }

    /// Outside the embedded kernels, the fast path errs exactly like the
    /// almanac (no extrapolation) - the frames panic contract depends on
    /// this.
    #[test]
    fn outside_span_errs() {
        let data = test_data();
        let epoch = Epoch::from_gregorian_utc(1950, 1, 1, 0, 0, 0, 0);
        assert!(data.earth_rotation.dcm_j2000_to_itrf93(epoch).is_err());
        let epoch = Epoch::from_gregorian_utc(1500, 1, 1, 0, 0, 0, 0);
        assert!(
            data.ephemeris_segments
                .state_km(Body::Luna.naif_id(), 399, epoch)
                .is_err()
        );
    }
}
