use glam::{Mat4, Quat, Vec3};

use super::earth;

/// Orbital camera anchored to a look-at point on the globe surface.
///
/// World space is in kilometers with the planet center at the origin; the
/// surface is the WGS84 ellipsoid (see `earth`). The distance/near/far
/// constants below are the previously-tuned "globe radii" values scaled by
/// `earth::MEAN_RADIUS_KM`, so the interaction feel is unchanged.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Longitude of the look-at point, in degrees.
    pub longitude: f32,
    /// Latitude of the look-at point, in degrees. Clamped to +/-89 deg.
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
    // Near/far planes, in km (~0.01 and ~50 Earth radii). No depth buffer is
    // used, so these only bound clipping; the star shell must fit inside far.
    const NEAR_PLANE: f32 = 0.01 * earth::MEAN_RADIUS_KM;
    const FAR_PLANE: f32 = 50.0 * earth::MEAN_RADIUS_KM;
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

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let (eye, target, up) = self.frame();

        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(
            Self::FOV_Y.to_radians(),
            aspect.max(0.01),
            Self::NEAR_PLANE,
            Self::FAR_PLANE,
        );

        proj * view
    }

    /// The camera position in world space.
    pub fn eye(&self) -> Vec3 {
        self.frame().0
    }

    /// Computes the camera's (eye, target, up) in world space (km).
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
