use bytemuck::{Pod, Zeroable};

use super::earth;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    /// WGS84 ellipsoid surface position, in kilometers (planet center at the
    /// origin).
    pub position: [f32; 3],
    /// Outward geodetic unit normal at this vertex. Stored explicitly
    /// because on an ellipsoid the normal is no longer `normalize(position)`.
    /// It also doubles as a unit direction the atmosphere/star passes scale
    /// up into their spherical shells.
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Generates a WGS84 reference-ellipsoid globe, in kilometers.
///
/// `u` maps longitude (-180 deg at u=0 to +180 deg at u=1) and `v` maps
/// geodetic latitude from the north pole (v=0) to the south pole (v=1),
/// matching an equirectangular texture. The seam column at u=0/u=1 is
/// duplicated so the texture can wrap. Longitude 0 deg, latitude 0 deg faces
/// +Z; +Y is north. Vertex positions are on the oblate WGS84 ellipsoid; the
/// stored normal is the geodetic (surface) normal, which has the same lat/lon
/// direction as a sphere would.
pub fn wgs84_ellipsoid(stacks: u32, slices: u32) -> Mesh {
    let mut vertices = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);

    for i in 0..=stacks {
        let v = i as f32 / stacks as f32;
        let lat = (90.0 - 180.0 * v).to_radians();

        for j in 0..=slices {
            let u = j as f32 / slices as f32;
            let lon = (360.0 * u - 180.0).to_radians();

            vertices.push(Vertex {
                position: earth::surface_position(lat, lon).to_array(),
                normal: earth::geodetic_normal(lat, lon).to_array(),
                uv: [u, v],
            });
        }
    }

    let cols = slices + 1;
    let mut indices = Vec::with_capacity((stacks * slices * 6) as usize);

    for i in 0..stacks {
        for j in 0..slices {
            let a = i * cols + j;
            let b = (i + 1) * cols + j;
            let c = i * cols + j + 1;
            let d = (i + 1) * cols + j + 1;

            // Counter-clockwise when viewed from outside the ellipsoid.
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }

    Mesh { vertices, indices }
}
