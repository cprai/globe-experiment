use glam::{Mat4, Quat, Vec3};

/// Orbital camera anchored to a look-at point on the globe surface.
///
/// The globe is a unit sphere at the origin; distances are in globe radii.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Longitude of the look-at point, in degrees.
    pub longitude: f32,
    /// Latitude of the look-at point, in degrees. Clamped to ±89°.
    pub latitude: f32,
    /// Distance from the camera to the look-at point, in globe radii.
    pub distance: f32,
    /// Angle off straight-down (nadir), in degrees. 0 looks straight down.
    pub tilt: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            longitude: 0.0,
            latitude: 0.0,
            distance: 2.0,
            tilt: 0.0,
        }
    }
}

impl Camera {
    const FOV_Y: f32 = 45.0;
    const MIN_DISTANCE: f32 = 0.01;
    const MAX_DISTANCE: f32 = 10.0;
    const MAX_TILT: f32 = 80.0;

    /// Moves the look-at point by the given degrees, wrapping longitude
    /// across the dateline and clamping latitude short of the poles.
    pub fn pan(&mut self, dlon: f32, dlat: f32) {
        self.longitude =
            (self.longitude + dlon + 180.0).rem_euclid(360.0) - 180.0;
        self.latitude = (self.latitude + dlat).clamp(-89.0, 89.0);
    }

    /// Scales the camera distance, clamped between just above the surface
    /// and a full-globe view.
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor)
            .clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Adjusts the tilt, clamped between straight-down and near-horizon.
    pub fn tilt_by(&mut self, degrees: f32) {
        self.tilt = (self.tilt + degrees).clamp(0.0, Self::MAX_TILT);
    }

    /// Degrees of arc panned per pixel of cursor movement, scaled so the
    /// ground under the cursor approximately follows it at any altitude.
    pub fn pan_degrees_per_pixel(&self, viewport_height: f32) -> f32 {
        let world_per_pixel = 2.0
            * self.distance
            * (Self::FOV_Y / 2.0).to_radians().tan()
            / viewport_height.max(1.0);

        // The globe is a unit sphere, so a world-unit of ground distance is
        // one radian of arc.
        world_per_pixel.to_degrees()
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        let (eye, target, up) = self.frame();

        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(
            Self::FOV_Y.to_radians(),
            aspect.max(0.01),
            0.01,
            50.0,
        );

        proj * view
    }

    /// The camera position in world space.
    pub fn eye(&self) -> Vec3 {
        self.frame().0
    }

    /// Computes the camera's (eye, target, up) in world space.
    fn frame(&self) -> (Vec3, Vec3, Vec3) {
        let lat = self.latitude.clamp(-89.0, 89.0).to_radians();
        let lon = self.longitude.to_radians();

        // Look-at point on the unit sphere surface; also the radial (up
        // from surface) direction.
        let radial = Vec3::new(
            lat.cos() * lon.sin(),
            lat.sin(),
            lat.cos() * lon.cos(),
        );
        let target = radial;

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
