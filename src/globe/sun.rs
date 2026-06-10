use glam::Vec3;

/// Position of the sun, expressed as the subsolar point — the spot on the
/// globe where the sun is directly overhead.
///
/// This parameterization is meant for animating later:
/// - Time of day sweeps `longitude` westward through 360° per day
///   (solar noon at UTC hour `h` sits near `(12 - h) * 15°`).
/// - The season moves `latitude` between ±23.44° (the solar declination):
///   +23.44° at the June solstice, 0° at the equinoxes, -23.44° in December.
#[derive(Clone, Copy, Debug)]
pub struct Sun {
    /// Longitude of the subsolar point, in degrees.
    pub longitude: f32,
    /// Latitude of the subsolar point, in degrees.
    pub latitude: f32,
}

impl Default for Sun {
    fn default() -> Self {
        // Morning over the Atlantic: lights the default camera pose
        // (0°N 0°E) from the upper left.
        Self {
            longitude: -40.0,
            latitude: 15.0,
        }
    }
}

impl Sun {
    /// Unit vector from the globe center toward the sun.
    pub fn direction(&self) -> Vec3 {
        let lat = self.latitude.to_radians();
        let lon = self.longitude.to_radians();

        Vec3::new(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos())
    }
}
