use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

use crate::earth;
use crate::simulation::CameraTarget;

/// Orbital camera that lives in the **inertial (star-fixed) frame** and orbits
/// a chosen body (the [`CameraTarget`] - Earth or the Moon).
///
/// The orbital math builds the rig (eye/target/up) around the *target's center*
/// exactly as a surface-anchored camera would, but that rig is interpreted in
/// the celestial frame and rotated into the Earth-fixed world frame at render
/// time (see `view_proj`). So the camera does not rotate with the world: it
/// holds still relative to the stars while the scene spins beneath it. The
/// longitude/latitude therefore select an inertial viewing direction, not a
/// fixed geographic point. With a Moon target the same construction keeps the
/// eye pinned to the Moon (look axis is always `-c2w * radial`, toward the Moon
/// center) while it tracks the Moon's moving ephemeris position.
///
/// World space is in kilometers with the Earth center at the origin; the
/// surface anchor and the distance/near/pan limits are scaled by the *target's*
/// mean radius (see [`CameraTarget::mean_radius_km`]), so the interaction feel
/// is the same fraction of whichever body is orbited. For an Earth target the
/// center is the origin, so the rig is identical to the original Earth-only
/// camera.
#[derive(Clone, Copy)]
pub struct Camera {
    /// Longitude of the inertial viewing direction, in degrees.
    pub longitude: f32,
    /// Latitude of the inertial viewing direction, in degrees. Clamped +/-89.
    pub latitude: f32,
    /// Distance from the camera to the look-at point, in kilometers.
    pub distance: f32,
    /// Angle off straight-down (nadir), in degrees. 0 looks straight down.
    pub tilt: f32,
    /// The body the rig orbits. Drives the surface anchor, the
    /// distance/near/pan limits, and the world-space center the rig is
    /// offset from.
    pub target: CameraTarget,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            longitude: 0.0,
            latitude: 0.0,
            // ~2 Earth radii above the surface: the whole body in view.
            distance: Self::DEFAULT_DISTANCE_RADII * earth::MEAN_RADIUS_KM,
            tilt: 0.0,
            target: CameraTarget::Earth,
        }
    }
}

impl Camera {
    const FOV_Y: f32 = 45.0;
    // Distance/near/default limits as fractions of the *target body's* mean
    // radius (was a hard-coded Earth radius). ~0.01 radii above the surface up to
    // ~10 radii out; the default view sits ~2 radii above the surface.
    const MIN_DISTANCE_RADII: f32 = 0.01;
    const MAX_DISTANCE_RADII: f32 = 10.0;
    const NEAR_PLANE_RADII: f32 = 0.01;
    const DEFAULT_DISTANCE_RADII: f32 = 2.0;
    // The far plane is a fixed 500,000 km (NOT a multiple of a body radius like
    // the other limits): it must enclose the Moon at lunar apogee (~406,700 km)
    // plus the camera's own distance, and - when orbiting the Moon - the Earth
    // ~384,400 km away; the star shell (222,985 km) sits well inside it.
    // Spanning ~17-64 km to 500,000 km is a ~4-orders-of-magnitude depth range,
    // which is why `view_proj` uses a reversed-Z projection (see there) - it
    // keeps depth precision usable across the whole range.
    const FAR_PLANE: f32 = 500_000.0;
    const MAX_TILT: f32 = 80.0;

    /// Closest the eye may sit above the target surface, in km.
    fn min_distance(&self) -> f32 {
        Self::MIN_DISTANCE_RADII * self.target.mean_radius_km()
    }

    /// Farthest the eye may sit from the target surface, in km.
    fn max_distance(&self) -> f32 {
        Self::MAX_DISTANCE_RADII * self.target.mean_radius_km()
    }

    /// Near clip plane for the target, in km.
    fn near_plane(&self) -> f32 {
        Self::NEAR_PLANE_RADII * self.target.mean_radius_km()
    }

    /// The full-body framing distance for the target, in km. Used when a body
    /// switch reframes the camera.
    fn default_distance(&self) -> f32 {
        Self::DEFAULT_DISTANCE_RADII * self.target.mean_radius_km()
    }

    /// Moves the look-at point by the given degrees, wrapping longitude
    /// across the dateline and clamping latitude short of the poles.
    pub fn pan(&mut self, dlon: f32, dlat: f32) {
        self.longitude = (self.longitude + dlon + 180.0).rem_euclid(360.0) - 180.0;
        self.latitude = (self.latitude + dlat).clamp(-89.0, 89.0);
    }

    /// Clamps a camera distance to lie between just above the target surface
    /// and a full-body view. An instance method because the limits scale
    /// with the current target's radius.
    pub fn clamp_distance(&self, distance: f32) -> f32 {
        distance.clamp(self.min_distance(), self.max_distance())
    }

    /// Adjusts the tilt, clamped between straight-down and near-horizon.
    pub fn tilt_by(&mut self, degrees: f32) {
        self.tilt = (self.tilt + degrees).clamp(0.0, Self::MAX_TILT);
    }

    /// Degrees of arc panned per pixel of cursor movement, scaled so the
    /// ground under the cursor approximately follows it at any altitude.
    pub fn pan_degrees_per_pixel(&self, viewport_height: f32) -> f32 {
        // Ground arc length spanned by one pixel at the target plane, in km.
        let km_per_pixel =
            2.0 * self.distance * (Self::FOV_Y / 2.0).to_radians().tan() / viewport_height.max(1.0);

        // Convert that arc length to an angle on the body: one radian of arc
        // subtends ~one mean radius of surface distance, so scaling by the
        // target's radius keeps the pan feel right on the Moon as on the Earth.
        (km_per_pixel / self.target.mean_radius_km()).to_degrees()
    }

    /// The view-projection matrix. `celestial_to_world` is the rotation from
    /// the inertial (star) frame the rig lives in to the Earth-fixed world
    /// frame the scene is drawn in (the inverse of the celestial sphere's
    /// world->celestial rotation); applying it keeps the camera fixed relative
    /// to the stars while the world rotates beneath it.
    pub fn view_proj(&self, aspect: f32, celestial_to_world: Mat3) -> Mat4 {
        // Built in the floating-origin (render) frame, where the orbited body
        // sits at the numerical origin. Computed WITHOUT forming the absolute
        // eye: a far planet's absolute position is billions of km, so
        // `absolute_eye - render_origin` in f32 cancels catastrophically (the
        // view translation snaps to ~hundreds of km and the camera jitters).
        // `world_frame_relative` keeps the math local instead. For Earth/Moon
        // (render_origin 0) this equals the absolute rig - bit-identical.
        let (eye, target, up) = self.world_frame_relative(celestial_to_world);

        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(
            Self::FOV_Y.to_radians(),
            aspect.max(0.01),
            self.near_plane(),
            Self::FAR_PLANE,
        );

        // Reversed-Z: remap clip depth so the near plane maps to 1 and the far
        // plane to 0 (paired with a depth buffer cleared to 0.0 and a `Greater`
        // depth test in the renderer). Across this scene's enormous near/far
        // span the floating-point depth buffer would otherwise have almost no
        // precision far from the camera (where the Moon is); reversed-Z spreads
        // the mantissa's precision evenly, so the Earth still occludes the Moon
        // cleanly. `z_clip' = w_clip - z_clip`, i.e. negate the proj Z row and
        // add the W row.
        let reverse_z = Mat4::from_cols(
            Vec4::new(1.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 1.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0, 0.0),
            Vec4::new(0.0, 0.0, 1.0, 1.0),
        );

        reverse_z * proj * view
    }

    /// The camera eye in the floating-origin (render) frame (km) - what the
    /// renderer uploads as `camera_pos` and uses to center the star shell.
    /// Precise for far targets (see `world_frame_relative`); for Earth/Moon
    /// (render_origin 0) it is the absolute eye.
    pub fn eye_relative(&self, celestial_to_world: Mat3) -> Vec3 {
        self.world_frame_relative(celestial_to_world).0
    }

    /// An inertial camera that orbits `target` and whose look axis points along
    /// `world_look` - a direction in the Earth-fixed world frame (e.g. toward
    /// the Sun's day side or toward the Moon) - viewed from `distance` km with
    /// no tilt. `star_rot_inv` is the celestial sphere's world->celestial
    /// (equatorial) rotation, mapping the world direction back into the
    /// inertial frame the rig is built in. Used by the eclipse scenarios to
    /// frame their event on launch; the camera stays fully interactive
    /// afterward.
    pub fn looking_toward(
        target: CameraTarget,
        star_rot_inv: Mat3,
        world_look: Vec3,
        distance: f32,
    ) -> Self {
        // The rig's look axis (toward the target) is `-radial`; resolved into
        // the world it is `-celestial_to_world * radial = -star_rot_inv^T *
        // radial`. Setting that equal to `world_look` gives
        // `radial = -(star_rot_inv * world_look)` - the inertial direction the
        // eye sits along.
        let radial = -(star_rot_inv * world_look.normalize_or_zero());
        let mut camera = Self {
            longitude: radial.x.atan2(radial.z).to_degrees(),
            latitude: radial.y.clamp(-1.0, 1.0).asin().to_degrees(),
            distance,
            tilt: 0.0,
            target,
        };
        camera.distance = camera.clamp_distance(distance);
        camera
    }

    /// Updates the camera's orbit target for this frame. The Moon center is
    /// refreshed every frame (it moves), so this is called each frame
    /// regardless. On a genuine **body switch** the camera reframes -
    /// resets to the body's full-frame distance and zero tilt, and for the
    /// Moon re-aims at the near side - and returns `true` so the caller can
    /// cancel any in-flight zoom or flick (which still targets the old
    /// body's scale). Keeping the existing longitude/latitude for an Earth
    /// switch is fine: any inertial direction frames the Earth at the
    /// origin.
    pub fn retarget(&mut self, target: CameraTarget, celestial_to_world: Mat3) -> bool {
        let switched = !self.target.same_kind(&target);
        self.target = target;

        if switched {
            self.distance = self.default_distance();
            self.tilt = 0.0;

            // Aim at the body: for an off-origin target (the Moon or a planet)
            // look toward its center from the camera, the same mapping
            // `looking_toward` uses with a world look direction of +center. An
            // Earth target keeps the existing longitude/latitude (any inertial
            // direction frames the Earth at the origin).
            let center = self.target.center_world();
            if center != Vec3::ZERO {
                let star_rot_inv = celestial_to_world.transpose();
                let radial = -(star_rot_inv * center.normalize_or_zero());
                self.longitude = radial.x.atan2(radial.z).to_degrees();
                self.latitude = radial.y.clamp(-1.0, 1.0).asin().to_degrees();
            }
        }

        switched
    }

    /// The rig in the **floating-origin (render) frame**: the inertial offsets
    /// rotated into the world and shifted by `(center - render_origin)`, so the
    /// orbited body sits at/near the numerical origin. Computed this way
    /// (rather than forming the absolute rig and subtracting
    /// `render_origin`) so a far planet never goes through a billions-of-km
    /// f32 value: for the orbited body `center == render_origin`, so the
    /// shift is exactly zero and the rig is just `celestial_to_world *
    /// offset` (local, precise). For Earth/Moon (render_origin 0) the shift
    /// is the body center, so this equals the absolute rig - bit-identical
    /// to the pre-planet renderer.
    fn world_frame_relative(&self, celestial_to_world: Mat3) -> (Vec3, Vec3, Vec3) {
        let (eye, target, up) = self.frame();
        let shift = self.target.center_world() - self.target.render_origin();
        (
            shift + celestial_to_world * eye,
            shift + celestial_to_world * target,
            celestial_to_world * up,
        )
    }

    /// Computes the camera's (eye, target, up) in the inertial (star) frame,
    /// as offsets from the target body's center.
    fn frame(&self) -> (Vec3, Vec3, Vec3) {
        let lat = self.latitude.clamp(-89.0, 89.0).to_radians();
        let lon = self.longitude.to_radians();

        // Look-at point on the target surface (km) and the local "up" - the
        // geodetic normal there, which the eye offsets along.
        let target = self.target.surface_position(lat, lon);
        let radial = self.target.geodetic_normal(lat, lon);

        // Local tangent frame at the look-at point.
        let east = Vec3::Y.cross(radial).normalize();
        let north = radial.cross(east);

        // Tilt swings the camera away from straight-down, around the local
        // east axis, so increasing tilt reveals the horizon to the north.
        let tilt = Quat::from_axis_angle(east, self.tilt.to_radians());
        let eye = target + tilt * radial * self.distance;
        let up = tilt * north;

        (eye, target, up)
    }
}
