//! A provider of two flat planes with exact geometry, so depth can be measured against the truth.
//! It ships no game data; it exists for `examples/depth_check.rs` and for trying the link without a
//! game.
//!
//! - `wall`: a square 4000 m wide in the XY plane at z = 0, from y = 0 up to y = 4000, facing +Z
//! - `ground`: a square 4000 m wide in the XZ plane at y = 0, facing +Y
//!
//! Each is 80 x 80 quads.

use rewrite_bridge as rb;
use rewrite_bridge::{AnimatedModel, AssetInfo, AssetKind, BoxError, Model, Provider};

pub struct Selftest;

const HALF: f32 = 2000.0;
const CELLS: usize = 80;

/// `CELLS` x `CELLS` quads over the square spanned by `origin + u * a + v * b` (u, v in 0..=1), with
/// a vertex per corner, so triangles are 50 m across rather than 4000 m: a game's triangles are small
/// and the depth interpolates the way it does on a real model.
fn grid(name: &str, origin: [f32; 3], a: [f32; 3], b: [f32; 3], normal: [f32; 3]) -> Model {
    let n = CELLS + 1;
    let mut positions = Vec::with_capacity(n * n);
    let mut uvs = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let (u, v) = (i as f32 / CELLS as f32, j as f32 / CELLS as f32);
            positions.push(std::array::from_fn(|k| origin[k] + u * a[k] + v * b[k]));
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::with_capacity(CELLS * CELLS * 6);
    for j in 0..CELLS {
        for i in 0..CELLS {
            let at = |di: usize, dj: usize| ((j + dj) * n + i + di) as u32;
            indices.extend([at(0, 0), at(1, 0), at(1, 1), at(0, 0), at(1, 1), at(0, 1)]);
        }
    }
    Model {
        name: name.into(),
        nodes: vec![rb::Node::new("root", None)],
        meshes: vec![rb::Mesh {
            name: name.into(),
            node: 0,
            material: 0,
            normals: vec![normal; positions.len()],
            positions,
            uvs,
            indices,
            ..rb::Mesh::default()
        }],
        materials: vec![rb::Material {
            name: "plain".into(),
            base_color_factor: [0.8, 0.8, 0.8, 1.0],
            metallic: 0.0,
            roughness: 1.0,
            double_sided: true,
            ..rb::Material::default()
        }],
        skins: vec![],
    }
}

impl Provider for Selftest {
    fn game(&self) -> &str {
        "bridge-guest selftest"
    }

    fn list(&self, filter: &str) -> Result<Vec<AssetInfo>, BoxError> {
        Ok(["wall", "ground"]
            .into_iter()
            .filter(|n| n.contains(filter))
            .map(|n| AssetInfo {
                id: n.into(),
                name: n.into(),
                kind: AssetKind::Prop,
            })
            .collect())
    }

    fn load(&self, id: &str) -> Result<Model, BoxError> {
        match id {
            // u along +x, v along +y: counter-clockwise seen from +Z
            "wall" => Ok(grid(
                "wall",
                [-HALF, 0.0, 0.0],
                [2.0 * HALF, 0.0, 0.0],
                [0.0, 2.0 * HALF, 0.0],
                [0.0, 0.0, 1.0],
            )),
            // u along +x, v along -z: counter-clockwise seen from +Y
            "ground" => Ok(grid(
                "ground",
                [-HALF, 0.0, HALF],
                [2.0 * HALF, 0.0, 0.0],
                [0.0, 0.0, -2.0 * HALF],
                [0.0, 1.0, 0.0],
            )),
            other => Err(format!("no such selftest asset: {other} (wall, ground)").into()),
        }
    }

    fn load_animated(&self, id: &str) -> Result<AnimatedModel, BoxError> {
        self.load(id).map(AnimatedModel::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_planes_are_valid_models() {
        for id in ["wall", "ground"] {
            Selftest.load(id).unwrap().validate().unwrap();
        }
        assert!(Selftest.load("nope").is_err());
    }

    #[test]
    fn the_planes_wind_toward_their_normals() {
        for id in ["wall", "ground"] {
            let m = Selftest.load(id).unwrap();
            let mesh = &m.meshes[0];
            let p = |i: u32| bevy::math::Vec3::from(mesh.positions[i as usize]);
            let n = (p(mesh.indices[1]) - p(mesh.indices[0]))
                .cross(p(mesh.indices[2]) - p(mesh.indices[0]));
            assert!(n.dot(bevy::math::Vec3::from(mesh.normals[0])) > 0.0, "{id}");
        }
    }
}
