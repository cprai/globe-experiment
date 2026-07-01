use glam::{Mat3, Quat, Vec3};

use crate::simulation::CameraTarget;
use crate::simulation::celestial_sphere::CelestialSphere;
use crate::terra;

/// Orbital camera that lives in the **inertial (star-fixed) frame** and orbits
/// a chosen subject (the [`CameraTarget`] - Terra, Luna, a planet, or a free
/// point).
///
/// The orbital math builds the rig (eye/target/up) around the *target's center*
/// exactly as a surface-anchored camera would, but that rig is interpreted in
/// the celestial frame and rotated into the Earth-fixed world frame (see
/// `world_rig`; the renderer then builds the projection). So the camera does
/// not rotate with the world: it
/// holds still relative to the stars while the scene spins beneath it. The
/// longitude/latitude therefore select an inertial viewing direction, not a
/// fixed geographic point. With a Luna target the same construction keeps the
/// eye pinned to Luna (look axis is always `-c2w * radial`, toward Luna
/// center) while it tracks Luna's moving ephemeris position.
///
/// World space is in kilometers with the Terra center at the origin; the
/// surface anchor and the distance/near/pan limits are scaled by the *target's*
/// mean radius (see [`CameraTarget::mean_radius_km`]), so the interaction feel
/// is the same fraction of whichever body is orbited. For a Terra target the
/// center is the origin, so the rig is identical to the original Terra-only
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
            // ~2 Terra radii above the surface: the whole body in view.
            distance: Self::DEFAULT_DISTANCE_RADII * terra::MEAN_RADIUS_KM,
            tilt: 0.0,
            target: CameraTarget::terra(),
        }
    }
}

impl Camera {
    // Distance/default limits as fractions of the *target body's* mean radius
    // (was a hard-coded Terra radius). ~0.01 radii above the surface up to ~10
    // radii out; the default view sits ~2 radii above the surface. The FOV, near
    // plane, and far plane now live with the projection in `renderer` (the
    // renderer rebuilds view_proj from the rig this camera emits).
    const MIN_DISTANCE_RADII: f32 = 0.01;
    const MAX_DISTANCE_RADII: f32 = 10.0;
    const DEFAULT_DISTANCE_RADII: f32 = 2.0;
    const MAX_TILT: f32 = 80.0;

    /// Closest the eye may sit above the target surface, in km.
    fn min_distance(&self) -> f32 {
        Self::MIN_DISTANCE_RADII * self.target.mean_radius_km()
    }

    /// Farthest the eye may sit from the target surface, in km.
    fn max_distance(&self) -> f32 {
        Self::MAX_DISTANCE_RADII * self.target.mean_radius_km()
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
            2.0 * self.distance * (crate::renderer::FOV_Y_DEG / 2.0).to_radians().tan()
                / viewport_height.max(1.0);

        // Convert that arc length to an angle on the body: one radian of arc
        // subtends ~one mean radius of surface distance, so scaling by the
        // target's radius keeps the pan feel right on Luna as on Terra.
        (km_per_pixel / self.target.mean_radius_km()).to_degrees()
    }

    /// The camera rig for the renderer: the eye, the look-at point, and the up
    /// vector, all in the floating-origin (render) frame.
    /// `celestial_to_world` is the rotation from the inertial (star) frame the
    /// rig lives in to the Earth-fixed world frame the scene is drawn in (the
    /// inverse of the celestial sphere's world->celestial rotation); applying
    /// it keeps the camera fixed relative to the stars while the world
    /// rotates beneath it.
    ///
    /// The renderer rebuilds the view-projection matrix from these (see
    /// `renderer::view_proj_reversed_z`). The look direction is carried as the
    /// look-at *point* rather than a normalized forward vector so the
    /// renderer's `look_at_rh` reproduces the old `view_proj` bit-for-bit
    /// (re-normalizing a forward and re-projecting would drift by sub-ULP
    /// and speckle every antialiased edge). The eye is kept in the render
    /// frame - computed WITHOUT ever forming the absolute eye - because a
    /// far planet's absolute position is billions of km, so `absolute_eye -
    /// render_origin` in f32 would cancel catastrophically (the view
    /// translation snaps to ~hundreds of km and the camera jitters). For
    /// Terra/Luna (render_origin 0) the render frame is the absolute frame.
    pub fn world_rig(
        &self,
        celestial: &CelestialSphere,
        celestial_to_world: Mat3,
    ) -> (Vec3, Vec3, Vec3) {
        self.world_frame_relative(celestial, celestial_to_world)
    }

    /// An inertial camera that orbits `target` and whose look axis points along
    /// `world_look` - a direction in the Earth-fixed world frame (e.g. toward
    /// Sol's day side or toward Luna) - viewed from `distance` km with
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

    /// Updates the camera's orbit target for this frame. The Luna center is
    /// refreshed every frame (it moves), so this is called each frame
    /// regardless. On a genuine **body switch** the camera reframes -
    /// resets to the body's full-frame distance and zero tilt, and for the
    /// Luna re-aims at the near side - and returns `true` so the caller can
    /// cancel any in-flight zoom or flick (which still targets the old
    /// body's scale). Keeping the existing longitude/latitude for a Terra
    /// switch is fine: any inertial direction frames Terra at the
    /// origin.
    pub fn retarget(
        &mut self,
        target: CameraTarget,
        celestial: &CelestialSphere,
        celestial_to_world: Mat3,
    ) -> bool {
        let switched = !self.target.same_kind(&target);
        self.target = target;

        if switched {
            self.distance = self.default_distance();
            self.tilt = 0.0;

            // Aim at the body: for an off-origin target (Luna or a planet)
            // look toward its center from the camera, the same mapping
            // `looking_toward` uses with a world look direction of +center. An
            // Terra target keeps the existing longitude/latitude (any inertial
            // direction frames Terra at the origin).
            let center = self.target.center_world(celestial);
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
    /// offset` (local, precise). For Terra/Luna (render_origin 0) the shift
    /// is the body center, so this equals the absolute rig - bit-identical
    /// to the pre-planet renderer.
    fn world_frame_relative(
        &self,
        celestial: &CelestialSphere,
        celestial_to_world: Mat3,
    ) -> (Vec3, Vec3, Vec3) {
        let (eye, target, up) = self.frame();
        let shift = self.target.center_world(celestial) - self.target.render_origin(celestial);
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
