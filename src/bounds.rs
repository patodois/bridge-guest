//! Where a model's feet are. The guest stands a model on the host's ground, so it needs the lowest
//! point, and for `--in-front` the middle of its footprint.

use bevy::math::{Mat4, Quat, Vec3};
use rewrite_bridge as rb;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Half the diagonal of the footprint, metres
    pub fn footprint_radius(&self) -> f32 {
        let d = self.max - self.min;
        0.5 * (d.x * d.x + d.z * d.z).sqrt()
    }

    pub fn scaled(&self, s: f32) -> Self {
        Self {
            min: self.min * s,
            max: self.max * s,
        }
    }
}

/// The model's axis-aligned bounds at rest, in the model's own space: rigid meshes at their node's
/// world transform, skinned meshes where their vertices are (a skin at rest leaves them in place).
pub fn rest_bounds(model: &rb::Model) -> Option<Bounds> {
    let mut world: Vec<Mat4> = Vec::with_capacity(model.nodes.len());
    for n in &model.nodes {
        let local = Mat4::from_scale_rotation_translation(
            Vec3::from(n.scale),
            Quat::from_xyzw(n.rotation[0], n.rotation[1], n.rotation[2], n.rotation[3]).normalize(),
            Vec3::from(n.translation),
        );
        world.push(match n.parent {
            Some(p) if p < world.len() => world[p] * local,
            _ => local,
        });
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for mesh in &model.meshes {
        let m = if mesh.skin.is_some() {
            Mat4::IDENTITY
        } else {
            world.get(mesh.node).copied().unwrap_or(Mat4::IDENTITY)
        };
        for p in &mesh.positions {
            let p = m.transform_point3(Vec3::from(*p));
            if p.is_finite() {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
    }
    lo.is_finite().then_some(Bounds { min: lo, max: hi })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rigid_mesh_follows_its_node() {
        let mut root = rb::Node::new("root", None);
        root.translation = [10.0, 2.0, 0.0];
        let mut child = rb::Node::new("child", Some(0));
        child.scale = [2.0, 2.0, 2.0];
        let model = rb::Model {
            name: "m".into(),
            nodes: vec![root, child],
            meshes: vec![rb::Mesh {
                node: 1,
                positions: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
                ..rb::Mesh::default()
            }],
            ..rb::Model::default()
        };
        let b = rest_bounds(&model).unwrap();
        assert_eq!(b.min, Vec3::new(10.0, 2.0, 0.0));
        assert_eq!(b.max, Vec3::new(12.0, 4.0, 2.0));
        assert!((b.footprint_radius() - 2.0_f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn a_model_without_vertices_has_no_bounds() {
        assert!(rest_bounds(&rb::Model::default()).is_none());
    }
}
