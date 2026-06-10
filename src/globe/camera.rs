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

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
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

        let view = Mat4::look_at_rh(eye, target, up);
        let proj = Mat4::perspective_rh(
            Self::FOV_Y.to_radians(),
            aspect.max(0.01),
            0.01,
            50.0,
        );

        proj * view
    }
}
