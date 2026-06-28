use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

use crate::earth;

/// Orbital camera that lives in the **inertial (star-fixed) frame**.
///
/// The orbital math builds the rig (eye/target/up) around the origin exactly
/// as a surface-anchored camera would, but that rig is interpreted in the
/// celestial frame and rotated into the Earth-fixed world frame at render time
/// (see `view_proj`). So the camera does not rotate with the Earth: it holds
/// still relative to the stars while the globe spins beneath it. The
/// longitude/latitude therefore select an inertial viewing direction, not a
/// fixed geographic point.
///
/// World space is in kilometers with the planet center at the origin; the
/// surface is the WGS84 ellipsoid (see `earth`). The distance/near/far
/// constants below are the previously-tuned "globe radii" values scaled by
/// `earth::MEAN_RADIUS_KM`, so the interaction feel is unchanged.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Longitude of the inertial viewing direction, in degrees.
    pub longitude: f32,
    /// Latitude of the inertial viewing direction, in degrees. Clamped +/-89.
    pub latitude: f32,
    /// Distance from the camera to the look-at point, in kilometers.
    pub distance: f32,
    /// Angle off straight-down (nadir), in degrees. 0 looks straight down.
    pub tilt: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            longitude: 0.0,
            latitude: 0.0,
            // ~2 Earth radii from the surface: a full-globe view.
            distance: 2.0 * earth::MEAN_RADIUS_KM,
            tilt: 0.0,
        }
    }
}

impl Camera {
    const FOV_Y: f32 = 45.0;
    // ~0.01 Earth radii above the surface up to ~10 radii out, in km.
    const MIN_DISTANCE: f32 = 0.01 * earth::MEAN_RADIUS_KM;
    const MAX_DISTANCE: f32 = 10.0 * earth::MEAN_RADIUS_KM;
    // Near plane, in km (~0.01 Earth radii). The far plane is a fixed 500,000
    // km (NOT a multiple of the Earth radius like the other limits): it must
    // enclose the Moon at lunar apogee (~406,700 km) plus the camera's own
    // distance (up to ~63,710 km), and the star shell (222,985 km) sits well
    // inside it. Spanning ~64 km to 500,000 km is a ~4-orders-of-magnitude
    // depth range, which is why `view_proj` uses a reversed-Z projection (see
    // there) - it keeps depth precision usable across the whole range.
    const NEAR_PLANE: f32 = 0.01 * earth::MEAN_RADIUS_KM;
    const FAR_PLANE: f32 = 500_000.0;
    const MAX_TILT: f32 = 80.0;

    /// Moves the look-at point by the given degrees, wrapping longitude
    /// across the dateline and clamping latitude short of the poles.
    pub fn pan(&mut self, dlon: f32, dlat: f32) {
        self.longitude = (self.longitude + dlon + 180.0).rem_euclid(360.0) - 180.0;
        self.latitude = (self.latitude + dlat).clamp(-89.0, 89.0);
    }

    /// Clamps a camera distance to lie between just above the surface and
    /// a full-globe view.
    pub fn clamp_distance(distance: f32) -> f32 {
        distance.clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE)
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

        // Convert that arc length to an angle on the globe: one radian of
        // arc subtends ~one mean Earth radius of surface distance.
        (km_per_pixel / earth::MEAN_RADIUS_KM).to_degrees()
    }

    /// The view-projection matrix. `celestial_to_world` is the rotation from
    /// the inertial (star) frame the rig lives in to the Earth-fixed world
    /// frame the scene is drawn in (the inverse of the celestial sphere's
    /// world->celestial rotation); applying it keeps the camera fixed relative
    /// to the stars while the Earth rotates beneath it.
    pub fn view_proj(&self, aspect: f32, celestial_to_world: Mat3) -> Mat4 {
        let (eye, target, up) = self.world_frame(celestial_to_world);

        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(
            Self::FOV_Y.to_radians(),
            aspect.max(0.01),
            Self::NEAR_PLANE,
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

    /// The camera position in the Earth-fixed world frame (km).
    pub fn eye(&self, celestial_to_world: Mat3) -> Vec3 {
        self.world_frame(celestial_to_world).0
    }

    /// An inertial camera whose look axis points along `world_look` - a
    /// direction in the Earth-fixed world frame (e.g. toward the Sun's day side
    /// or the Moon) - viewed from `distance` km with no tilt. `star_rot_inv` is
    /// the celestial sphere's world->celestial (equatorial) rotation, mapping
    /// the world direction back into the inertial frame the rig is built in.
    /// Used by the eclipse scenarios to frame their event on launch; the camera
    /// stays fully interactive afterward.
    pub fn looking_toward(star_rot_inv: Mat3, world_look: Vec3, distance: f32) -> Self {
        // The rig's look axis (toward the target) is `-radial`; resolved into
        // the world it is `-celestial_to_world * radial = -star_rot_inv^T *
        // radial`. Setting that equal to `world_look` gives
        // `radial = -(star_rot_inv * world_look)` - the inertial direction the
        // eye sits along.
        let radial = -(star_rot_inv * world_look.normalize_or_zero());
        Self {
            longitude: radial.x.atan2(radial.z).to_degrees(),
            latitude: radial.y.clamp(-1.0, 1.0).asin().to_degrees(),
            distance: Self::clamp_distance(distance),
            tilt: 0.0,
        }
    }

    /// The rig rotated from the inertial frame into the Earth-fixed world
    /// frame for rendering. The rotation is about the origin, so points and
    /// directions transform alike.
    fn world_frame(&self, celestial_to_world: Mat3) -> (Vec3, Vec3, Vec3) {
        let (eye, target, up) = self.frame();
        (
            celestial_to_world * eye,
            celestial_to_world * target,
            celestial_to_world * up,
        )
    }

    /// Computes the camera's (eye, target, up) in the inertial (star) frame.
    fn frame(&self) -> (Vec3, Vec3, Vec3) {
        let lat = self.latitude.clamp(-89.0, 89.0).to_radians();
        let lon = self.longitude.to_radians();

        // Look-at point on the WGS84 surface (km) and the local "up" - the
        // geodetic normal there, which the eye offsets along.
        let target = earth::surface_position(lat, lon);
        let radial = earth::geodetic_normal(lat, lon);

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
