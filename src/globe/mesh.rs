use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// Generates a unit UV sphere.
///
/// `u` maps longitude (-180 deg at u=0 to +180 deg at u=1) and `v` maps latitude
/// from the north pole (v=0) to the south pole (v=1), matching an
/// equirectangular texture. The seam column at u=0/u=1 is duplicated so the
/// texture can wrap. Longitude 0 deg, latitude 0 deg faces +Z; +Y is north.
pub fn uv_sphere(stacks: u32, slices: u32) -> Mesh {
    let mut vertices = Vec::with_capacity(((stacks + 1) * (slices + 1)) as usize);

    for i in 0..=stacks {
        let v = i as f32 / stacks as f32;
        let lat = (90.0 - 180.0 * v).to_radians();

        for j in 0..=slices {
            let u = j as f32 / slices as f32;
            let lon = (360.0 * u - 180.0).to_radians();

            vertices.push(Vertex {
                position: [lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos()],
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

            // Counter-clockwise when viewed from outside the sphere.
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }

    Mesh { vertices, indices }
}
