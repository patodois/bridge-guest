//! The host's camera as a Bevy camera.
//!
//! A `Camera` datagram carries the view (world to view) and the projection (view to clip, +y up in
//! NDC) of the frame the host is about to present. The guest renders with exactly those two
//! matrices, so a pixel of the guest's image lies where the same direction lies in the host's.
//!
//! Position and orientation need no conversion: the link and Bevy are both right-handed, +Y up,
//! looking down the camera's -Z. The view's inverse is the camera's `Transform`.
//!
//! The projection keeps the host's rows for x, y and w (so field of view, aspect ratio and any
//! off-centre shift are the host's) and replaces its clip-depth row, whatever `depth_range` the host
//! uses (0 OpenGL, 1 Direct3D/Vulkan/Metal, 2 reversed), with the one Bevy and wgpu draw with:
//! reversed, infinite far plane, near 1 and infinity 0, `z_clip = near`, `w_clip = -z_view`. The
//! depth buffer then holds `near / t` for a surface `t` metres in front of the camera, which the depth
//! pass inverts exactly (`t = near / d`). The host's far plane is not drawn with (an infinite far
//! plane keeps reversed-Z precise out to the horizon); it is applied when the depth is read back.

use bevy::camera::{CameraProjection, SubCameraView};
use bevy::math::{Mat4, Vec3A, Vec4};
use bevy::prelude::*;
use rewrite_bridge::link::Camera as HostCamera;

/// What a bad camera is called in the log.
#[derive(Debug, PartialEq)]
pub enum Unsupported {
    NotPerspective,
    BadNear,
    NotFinite,
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPerspective => write!(
                f,
                "the host's projection is not a perspective one (w_clip is not -z_view)"
            ),
            Self::BadNear => write!(f, "the host's near plane is not a positive number"),
            Self::NotFinite => write!(f, "the host's camera holds a NaN or infinity"),
        }
    }
}

/// The projection of one host camera, in Bevy's convention. See the module docs.
#[derive(Clone, Copy, Debug)]
pub struct HostProjection {
    clip_from_view: Mat4,
    /// Host clip x/y offsets of the view axis, `proj[8]` and `proj[9]`: zero for a centred frustum
    shift: [f32; 2],
    scale: [f32; 2],
    /// Distance at which Bevy's frustum culling and shadow code stop looking, metres
    far_cull: f32,
    pub near: f32,
    /// The host's far plane, metres; infinity when it has none
    pub far: f32,
}

impl Default for HostProjection {
    fn default() -> Self {
        Self::from_host(&HostCamera::look_at(
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0],
            1.0,
            0.1,
            f32::INFINITY,
            [16, 9],
        ))
        .expect("the default camera is fine")
    }
}

impl HostProjection {
    /// The projection for `camera`, or why it cannot be used.
    pub fn from_host(camera: &HostCamera) -> Result<Self, Unsupported> {
        let p = &camera.proj;
        if p.iter().chain(&camera.view).any(|v| !v.is_finite()) {
            return Err(Unsupported::NotFinite);
        }
        // Perspective: w_clip = -z_view, and no depth in x or y
        if p[3] != 0.0 || p[7] != 0.0 || p[11] != -1.0 || p[15] != 0.0 {
            return Err(Unsupported::NotPerspective);
        }
        let near = camera.near;
        if !(near.is_finite() && near > 0.0) {
            return Err(Unsupported::BadNear);
        }
        // Columns of the host's matrix with row 2 (clip z) swapped for the reversed, infinite one:
        // z_clip = near (a = 0, b = near), so depth = near / w_clip.
        let clip_from_view = Mat4::from_cols(
            Vec4::new(p[0], p[1], 0.0, p[3]),
            Vec4::new(p[4], p[5], 0.0, p[7]),
            Vec4::new(p[8], p[9], 0.0, p[11]),
            Vec4::new(p[12], p[13], near, p[15]),
        );
        Ok(Self {
            clip_from_view,
            shift: [p[8], p[9]],
            scale: [p[0], p[5]],
            far_cull: if camera.far.is_finite() {
                camera.far
            } else {
                1.0e6
            },
            near,
            far: camera.far,
        })
    }
}

impl CameraProjection for HostProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        self.clip_from_view
    }

    fn get_clip_from_view_for_sub(&self, _sub_view: &SubCameraView) -> Mat4 {
        self.clip_from_view
    }

    /// The host's matrix fixes the aspect ratio; the render target is made to match it.
    fn update(&mut self, _width: f32, _height: f32) {}

    #[allow(clippy::misnamed_getters)]
    fn far(&self) -> f32 {
        self.far_cull
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        // x_clip = p0 x + p8 z, w_clip = -z: ndc_x = p0 x / t - p8, so x = t (ndc_x + p8) / p0
        let at = |ndc_x: f32, ndc_y: f32, z: f32| {
            let t = z.abs();
            Vec3A::new(
                t * (ndc_x + self.shift[0]) / self.scale[0],
                t * (ndc_y + self.shift[1]) / self.scale[1],
                z,
            )
        };
        // The order Bevy asks for: bottom right, top right, top left, bottom left; near, then far
        [
            at(1.0, -1.0, z_near),
            at(1.0, 1.0, z_near),
            at(-1.0, 1.0, z_near),
            at(-1.0, -1.0, z_near),
            at(1.0, -1.0, z_far),
            at(1.0, 1.0, z_far),
            at(-1.0, 1.0, z_far),
            at(-1.0, -1.0, z_far),
        ]
    }
}

/// The camera's `Transform`: the inverse of the host's view matrix.
pub fn transform_from_view(view: &[f32; 16]) -> Transform {
    Transform::from_matrix(Mat4::from_cols_array(view).inverse())
}

/// The size of the image to render for a host viewport: as large as `max` allows while keeping the
/// viewport's aspect ratio, so pixels stay square.
pub fn render_size(viewport: [u32; 2], max: (u32, u32)) -> (u32, u32) {
    let (vw, vh) = (viewport[0].max(1) as f64, viewport[1].max(1) as f64);
    let (mw, mh) = (max.0 as f64, max.1 as f64);
    let aspect = vw / vh;
    let (w, h) = if aspect >= mw / mh {
        (mw, (mw / aspect).round())
    } else {
        ((mh * aspect).round(), mh)
    };
    (w.max(8.0) as u32, h.max(8.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rewrite_bridge::link::DepthRange;

    fn camera(range: DepthRange, far: f32) -> HostCamera {
        HostCamera::look_at(
            [1.0, 2.0, 3.0],
            [0.0, 1.0, -4.0],
            1.0,
            0.1,
            far,
            [1280, 720],
        )
        .with_depth_range(range)
    }

    #[test]
    fn every_depth_range_gives_the_same_bevy_projection() {
        let a = HostProjection::from_host(&camera(DepthRange::NegOneToOne, 500.0)).unwrap();
        let b = HostProjection::from_host(&camera(DepthRange::ZeroToOne, 500.0)).unwrap();
        let c = HostProjection::from_host(&camera(DepthRange::Reversed, f32::INFINITY)).unwrap();
        assert_eq!(a.get_clip_from_view(), b.get_clip_from_view());
        assert_eq!(a.get_clip_from_view(), c.get_clip_from_view());
        assert_eq!(c.far, f32::INFINITY);
    }

    #[test]
    fn depth_is_near_over_distance() {
        let p = HostProjection::from_host(&camera(DepthRange::ZeroToOne, 500.0)).unwrap();
        for t in [0.1_f32, 1.0, 20.0, 400.0] {
            let clip = p.get_clip_from_view() * Vec4::new(0.3, -0.2, -t, 1.0);
            assert!((clip.z / clip.w - 0.1 / t).abs() < 1e-6 * (0.1 / t).max(1e-3));
        }
    }

    #[test]
    fn x_and_y_follow_the_host() {
        let host = camera(DepthRange::NegOneToOne, 100.0);
        let ours = HostProjection::from_host(&host)
            .unwrap()
            .get_clip_from_view();
        let theirs = Mat4::from_cols_array(&host.proj);
        for v in [
            Vec4::new(1.0, 2.0, -5.0, 1.0),
            Vec4::new(-3.0, 0.5, -0.4, 1.0),
        ] {
            let (a, b) = (ours * v, theirs * v);
            assert_eq!((a.x, a.y, a.w), (b.x, b.y, b.w));
        }
    }

    #[test]
    fn frustum_corners_project_back_to_the_ndc_corners() {
        let p = HostProjection::from_host(&camera(DepthRange::Reversed, 100.0)).unwrap();
        for c in p.get_frustum_corners(-0.1, -50.0) {
            let clip = p.get_clip_from_view() * Vec4::new(c.x, c.y, c.z, 1.0);
            assert!((clip.x / clip.w).abs() - 1.0 < 1e-5);
            assert!((clip.y / clip.w).abs() - 1.0 < 1e-5);
        }
    }

    #[test]
    fn view_to_transform_round_trips() {
        let host = camera(DepthRange::ZeroToOne, 100.0);
        let t = transform_from_view(&host.view);
        let eye = host.eye();
        assert!((t.translation - Vec3::from(eye)).length() < 1e-5);
        let f = host.forward();
        assert!((t.forward().as_vec3() - Vec3::from(f)).length() < 1e-5);
    }

    #[test]
    fn the_image_keeps_the_viewports_aspect() {
        assert_eq!(render_size([1920, 1080], (640, 360)), (640, 360));
        assert_eq!(render_size([1280, 800], (640, 360)), (576, 360));
        assert_eq!(render_size([800, 1000], (640, 360)), (288, 360));
        assert_eq!(render_size([3440, 1440], (640, 360)), (640, 268));
    }

    #[test]
    fn orthographic_and_broken_cameras_are_refused() {
        let mut host = camera(DepthRange::ZeroToOne, 100.0);
        host.proj[11] = 0.0;
        host.proj[15] = 1.0;
        assert_eq!(
            HostProjection::from_host(&host).unwrap_err(),
            Unsupported::NotPerspective
        );
        let mut host = camera(DepthRange::ZeroToOne, 100.0);
        host.near = 0.0;
        assert_eq!(
            HostProjection::from_host(&host).unwrap_err(),
            Unsupported::BadNear
        );
        let mut host = camera(DepthRange::ZeroToOne, 100.0);
        host.view[3] = f32::NAN;
        assert_eq!(
            HostProjection::from_host(&host).unwrap_err(),
            Unsupported::NotFinite
        );
    }
}
