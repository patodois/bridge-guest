//! Where the model stands in the host's world, and its pose.
//!
//! Without `--in-front` the model stands at the world origin, as `fake_guest` does. With it, the
//! model's footprint is centred a few metres ahead of the first camera and it faces the camera. It
//! stands at eye height minus 1.6 m until a `Collision` snapshot covers the spot, then on the host's
//! ground under it (the highest collision triangle there that is not above the eye), as `fake_guest`
//! does. The pose sent is the model root's: the model is lifted so that its lowest point touches the
//! ground, whatever the provider chose for its origin.

use bevy::math::{Quat, Vec3};
use rewrite_bridge::link::{Camera, Pose, Triangle};

use crate::bounds::Bounds;

/// How far below the eye the ground is guessed to be before the host's collision says
pub const EYE_HEIGHT: f32 = 1.6;
/// How far ahead of the camera `--in-front` stands, at least, metres
pub const MIN_DISTANCE: f32 = 4.0;

pub struct PlacementOptions {
    pub in_front: bool,
    pub distance: Option<f32>,
    /// Radians about +Y; `None` = face the camera (--in-front) or +Z
    pub yaw: Option<f32>,
    pub scale: f32,
}

/// Where `--in-front` put the model, once a camera is known
#[derive(Clone, Copy, Debug)]
struct Anchor {
    /// Centre of the footprint
    x: f32,
    z: f32,
    eye_y: f32,
    ground_y: f32,
    grounded: bool,
    yaw: f32,
}

pub struct Placement {
    opts: PlacementOptions,
    /// At the scale the model is spawned with
    bounds: Bounds,
    anchor: Option<Anchor>,
    teleport: bool,
}

impl Placement {
    pub fn new(opts: PlacementOptions, rest_bounds: Bounds) -> Self {
        let bounds = rest_bounds.scaled(opts.scale);
        let mut p = Self {
            opts,
            bounds,
            anchor: None,
            teleport: true,
        };
        p.reset();
        p
    }

    /// Back to the start, for a new session
    pub fn reset(&mut self) {
        self.teleport = true;
        self.anchor = (!self.opts.in_front).then_some(Anchor {
            x: 0.0,
            z: 0.0,
            eye_y: 0.0,
            ground_y: 0.0,
            grounded: true,
            yaw: self.opts.yaw.unwrap_or(0.0),
        });
    }

    pub fn is_placed(&self) -> bool {
        self.anchor.is_some()
    }

    /// `Some(message)` when this camera decided where the model stands
    pub fn on_camera(&mut self, camera: &Camera) -> Option<String> {
        if self.anchor.is_some() {
            return None;
        }
        let (eye, f) = (camera.eye(), camera.forward());
        let len = (f[0] * f[0] + f[2] * f[2]).sqrt();
        let (dx, dz) = if len > 1e-3 {
            (f[0] / len, f[2] / len)
        } else {
            (0.0, -1.0)
        };
        let distance = self
            .opts
            .distance
            .unwrap_or_else(|| MIN_DISTANCE.max(1.5 * self.bounds.footprint_radius()));
        let (x, z) = (eye[0] + dx * distance, eye[2] + dz * distance);
        let yaw = self.opts.yaw.unwrap_or_else(|| (-dx).atan2(-dz));
        self.anchor = Some(Anchor {
            x,
            z,
            eye_y: eye[1],
            ground_y: eye[1] - EYE_HEIGHT,
            grounded: false,
            yaw,
        });
        self.teleport = true;
        Some(format!(
            "model placed {distance:.1} m ahead of the camera, at ({x:.2}, {z:.2}), facing {:.0} deg",
            yaw.to_degrees()
        ))
    }

    /// `Some(message)` when this snapshot put the model on the host's ground
    pub fn on_collision(&mut self, triangles: &[Triangle]) -> Option<String> {
        let a = self.anchor.as_mut()?;
        if a.grounded {
            return None;
        }
        let y = ground_under(triangles, a.x, a.z, a.eye_y)?;
        a.ground_y = y;
        a.grounded = true;
        self.teleport = true;
        Some(format!("model stands on the host's ground at y {y:.2}"))
    }

    /// The root's translation and yaw, or `None` before the model has a place
    fn root(&self) -> Option<(Vec3, f32)> {
        let a = self.anchor?;
        let y = a.ground_y - self.bounds.min.y;
        // --in-front centres the footprint on the spot; the origin placement leaves the model's
        // own origin there
        let (x, z) = if self.opts.in_front {
            let c = self.bounds.center();
            let (s, co) = a.yaw.sin_cos();
            (a.x - (c.x * co + c.z * s), a.z - (-c.x * s + c.z * co))
        } else {
            (a.x, a.z)
        };
        Some((Vec3::new(x, y, z), a.yaw))
    }

    /// The root's translation, rotation and scale for the Bevy world
    pub fn transform(&self) -> Option<bevy::prelude::Transform> {
        let (t, yaw) = self.root()?;
        Some(bevy::prelude::Transform {
            translation: t,
            rotation: Quat::from_rotation_y(yaw),
            scale: Vec3::splat(self.opts.scale),
        })
    }

    /// The pose to send now for entity `entity`; marks the next one as no teleport
    pub fn pose(&mut self, entity: u32, time_us: u64) -> Option<Pose> {
        let (t, yaw) = self.root()?;
        let mut pose = Pose::new(entity, time_us);
        pose.translation = t.to_array();
        let q = Quat::from_rotation_y(yaw);
        pose.rotation = [q.x, q.y, q.z, q.w];
        pose.scale = [self.opts.scale; 3];
        if std::mem::take(&mut self.teleport) {
            pose.flags |= rewrite_bridge::link::POSE_TELEPORT;
        }
        Some(pose)
    }
}

/// The height of the highest collision triangle under `(x, z)` that is not above `max_y`.
pub fn ground_under(triangles: &[Triangle], x: f32, z: f32, max_y: f32) -> Option<f32> {
    triangles
        .iter()
        .filter_map(|[a, b, c]| {
            // Barycentric coordinates of (x, z) in the triangle projected onto the ground plane
            let det = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
            if det.abs() < 1e-9 {
                return None;
            }
            let u = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / det;
            let v = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / det;
            let w = 1.0 - u - v;
            (u >= 0.0 && v >= 0.0 && w >= 0.0).then(|| u * a[1] + v * b[1] + w * c[1])
        })
        .filter(|&y| y <= max_y)
        .max_by(f32::total_cmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds {
        // 2 m wide, 1 m deep, 3 m tall, its origin 1 m above its feet and 0.5 m off-centre in x
        Bounds {
            min: Vec3::new(-0.5, -1.0, -0.5),
            max: Vec3::new(1.5, 2.0, 0.5),
        }
    }

    fn opts(in_front: bool) -> PlacementOptions {
        PlacementOptions {
            in_front,
            distance: None,
            yaw: None,
            scale: 1.0,
        }
    }

    #[test]
    fn the_origin_placement_lifts_the_feet_to_zero() {
        let mut p = Placement::new(opts(false), bounds());
        let pose = p.pose(1, 5).unwrap();
        assert_eq!(pose.translation, [0.0, 1.0, 0.0]);
        assert_eq!(pose.flags & 1, 1, "the first pose teleports");
        assert_eq!(p.pose(1, 6).unwrap().flags & 1, 0);
    }

    #[test]
    fn in_front_waits_for_a_camera_and_stands_ahead_facing_it() {
        let mut p = Placement::new(opts(true), bounds());
        assert!(p.pose(1, 0).is_none());
        // Eye at (10, 1.7, 10) looking toward -z
        let cam = Camera::look_at(
            [10.0, 1.7, 10.0],
            [10.0, 1.7, 0.0],
            1.0,
            0.1,
            100.0,
            [16, 9],
        );
        assert!(p.on_camera(&cam).is_some());
        let t = p.transform().unwrap();
        // Footprint centre 4 m ahead; the model faces the camera, so +z points back at it: yaw 0
        let c = bounds().center();
        let centre = t.translation + t.rotation * c;
        assert!(
            (centre.x - 10.0).abs() < 1e-4 && (centre.z - 6.0).abs() < 1e-4,
            "{centre:?}"
        );
        assert!(t.rotation.angle_between(Quat::IDENTITY) < 1e-4);
        // Until the host's collision says otherwise the feet are 1.6 m under the eye
        assert!((t.translation.y + bounds().min.y - 0.1).abs() < 1e-4);
        // A second camera changes nothing
        let other = Camera::look_at([0.0, 5.0, 0.0], [1.0, 5.0, 0.0], 1.0, 0.1, 100.0, [16, 9]);
        assert!(p.on_camera(&other).is_none());
    }

    #[test]
    fn collision_puts_the_feet_on_the_floor_under_the_spot() {
        let mut p = Placement::new(opts(true), bounds());
        let cam = Camera::look_at([0.0, 1.7, 0.0], [0.0, 1.7, -1.0], 1.0, 0.1, 100.0, [16, 9]);
        p.on_camera(&cam);
        // The spot is (0, -4). A floor at y 0.5 there, a ceiling at 3 (above the eye), a pit at -2
        let tri = |y: f32| -> Triangle { [[-10.0, y, -10.0], [10.0, y, -10.0], [0.0, y, 10.0]] };
        assert!(p.on_collision(&[]).is_none());
        assert!(
            p.on_collision(&[tri(3.0)]).is_none(),
            "above the eye is not ground"
        );
        assert!(p.on_collision(&[tri(-2.0), tri(0.5), tri(3.0)]).is_some());
        let t = p.transform().unwrap();
        assert!((t.translation.y + bounds().min.y - 0.5).abs() < 1e-4);
        // Once grounded a later snapshot does not move it
        assert!(p.on_collision(&[tri(0.0)]).is_none());
    }

    #[test]
    fn scale_and_yaw_apply() {
        let mut p = Placement::new(
            PlacementOptions {
                in_front: false,
                distance: None,
                yaw: Some(std::f32::consts::FRAC_PI_2),
                scale: 2.0,
            },
            bounds(),
        );
        let pose = p.pose(1, 0).unwrap();
        assert_eq!(pose.scale, [2.0; 3]);
        assert_eq!(pose.translation[1], 2.0);
        let q = Quat::from_xyzw(
            pose.rotation[0],
            pose.rotation[1],
            pose.rotation[2],
            pose.rotation[3],
        );
        assert!(((q * Vec3::Z) - Vec3::X).length() < 1e-5);
    }

    #[test]
    fn a_big_model_stands_further_away() {
        let big = Bounds {
            min: Vec3::new(-5.0, 0.0, -5.0),
            max: Vec3::new(5.0, 3.0, 5.0),
        };
        let mut p = Placement::new(opts(true), big);
        let cam = Camera::look_at([0.0, 1.7, 0.0], [0.0, 1.7, -1.0], 1.0, 0.1, 100.0, [16, 9]);
        let msg = p.on_camera(&cam).unwrap();
        assert!(msg.contains("10.6 m"), "{msg}");
    }
}
