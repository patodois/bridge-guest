//! The Bevy side: the model, the camera that draws it with the host's view, and the loop that
//! answers every host camera with a frame.
//!
//! ```text
//! host Camera ──> drive() ──> camera Transform + HostProjection ──> main pass ──> colour image ─┐
//!                                                        └─> depth buffer ──> LinearDepth pass ─┼─> Readback
//!                                                                              (R32Float image) ─┘      │
//! host <── Frame <── FrameWriter (pixel file, colour + depth) <── finish_frames() <── ReadbackComplete ◄┘
//! ```
//!
//! Nothing waits for the GPU: a camera is turned into a render and two read-back requests in one
//! frame of the app, and the frame is written when both read-backs have come back, a frame or two
//! later. At most two renders are in flight; a camera that arrives while both are busy replaces
//! the one waiting (the host wants the newest view anyway).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::camera::{ClearColorConfig, Exposure, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::GlobalAmbientLight;
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use rewrite_bridge::link::{
    now_us, Camera as HostCamera, Event, FrameFileOptions, FrameWriter, Link, Spawn,
};
use rewrite_bridge_bevy::{Bridge, BridgeAnimations};

use crate::camera::{render_size, transform_from_view, HostProjection};
use crate::cli::Opts;
use crate::depth::LinearDepth;
use crate::frames::assemble;
use crate::placement::Placement;
use crate::stats::Stats;

/// The entity id the link knows the model by
const ENTITY: u32 = 1;
/// Renders that may be waiting for their read-back at once
const MAX_IN_FLIGHT: usize = 2;
/// A render whose read-back has not come back after this long is given up
const GIVE_UP: Duration = Duration::from_secs(2);
/// Frames of the app to let the model's meshes and textures reach the GPU before the first render
const WARM_UP_FRAMES: u32 = 12;
/// Frames to let a new render target settle before it is drawn into
const SETTLE_FRAMES: u32 = 3;
const POSE_INTERVAL: Duration = Duration::from_millis(16);

/// The sun, lux, and the ambient light, cd/m2: the same neutral stage as `bridge-testbed`
const SUN_LUX: f32 = 10_000.0;
const AMBIENT_CD_M2: f32 = 400.0;
const SUN_TOWARD: Vec3 = Vec3::new(0.45, 0.8, 0.55);

#[derive(Component)]
pub struct GuestCamera;

#[derive(Component)]
pub struct GuestModel;

#[derive(Component)]
pub struct Preview;

/// The model, until the startup system spawns it
#[derive(Resource)]
pub struct ModelSlot(pub Mutex<Option<rewrite_bridge::AnimatedModel>>);

#[derive(Resource)]
pub struct ShutdownFlag(pub Arc<AtomicBool>);

/// The two images the camera draws into; replaced when the host's aspect ratio changes
#[derive(Resource)]
pub struct Targets {
    pub size: (u32, u32),
    pub colour: Handle<Image>,
    pub depth: Handle<Image>,
    settle: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Plane {
    Colour,
    Depth,
}

#[derive(Component)]
struct Ticket {
    id: u64,
    plane: Plane,
}

struct Job {
    camera_seq: u32,
    received: Instant,
    host_time_us: u64,
    render_us: u64,
    requested: Instant,
    far: f32,
    size: (u32, u32),
    colour: Option<Vec<u8>>,
    depth: Option<Vec<u8>>,
    entities: [Entity; 2],
}

#[derive(Resource, Default)]
pub struct InFlight {
    next: u64,
    jobs: HashMap<u64, Job>,
}

struct PendingCamera {
    seq: u32,
    camera: HostCamera,
    received: Instant,
    host_time_us: u64,
}

/// The link and everything the guest remembers about the session
pub struct Guest {
    pub link: Link,
    pub opts: Opts,
    pub placement: Placement,
    pub stats: Arc<Mutex<Stats>>,
    /// The provider's game, for `spawn`
    pub game: String,
    pub model_name: String,
    writer: Option<FrameWriter>,
    camera: Option<PendingCamera>,
    last_pose: Instant,
    started: Instant,
    frames_seen: u32,
    exit_at: Option<Instant>,
    warned: Vec<String>,
}

/// `Guest` as a resource: the link's sockets and channels are `Send` but not `Sync`
#[derive(Resource)]
pub struct GuestRes(pub Mutex<Guest>);

impl Guest {
    pub fn new(
        link: Link,
        opts: Opts,
        placement: Placement,
        stats: Arc<Mutex<Stats>>,
        game: String,
        model_name: String,
    ) -> Self {
        Self {
            link,
            opts,
            placement,
            stats,
            game,
            model_name,
            writer: None,
            camera: None,
            last_pose: Instant::now(),
            started: Instant::now(),
            frames_seen: 0,
            exit_at: None,
            warned: Vec::new(),
        }
    }

    fn log(&self, msg: impl AsRef<str>) {
        println!(
            "[guest {:7.2}s] {}",
            self.started.elapsed().as_secs_f32(),
            msg.as_ref()
        );
    }

    fn warn_once(&mut self, msg: String) {
        if !self.warned.contains(&msg) {
            self.log(format!("warning: {msg}"));
            self.warned.push(msg);
        }
    }

    fn on_connected(&mut self, peer: &rewrite_bridge::link::PeerInfo) {
        self.log(format!(
            "connected: host \"{}\", protocol v{}, session {:#010x}",
            peer.game, peer.protocol, peer.session
        ));
        self.placement.reset();
        let spawn = Spawn {
            entity: ENTITY,
            provider: self.game.clone(),
            asset: self.opts.id.clone().unwrap_or_default(),
            name: Some(self.model_name.clone()),
        };
        if let Err(e) = self.link.spawn(&spawn) {
            self.log(format!("spawn failed: {e}"));
        }
        let (w, h) = self.opts.size;
        let tag = format!("guest-{:08x}", peer.session);
        match FrameWriter::create_temp_with(&tag, &FrameFileOptions::new(w, h, 3).with_depth()) {
            Ok(wr) => {
                let _ = self.link.announce_frames(wr.channel());
                self.log(format!("pixel file {w}x{h} with a depth plane"));
                self.writer = Some(wr);
            }
            Err(e) => self.log(format!("no pixel file: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------------------------

pub struct GuestPlugin {
    pub window: bool,
}

impl Plugin for GuestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InFlight>()
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: AMBIENT_CD_M2,
                affects_lightmapped_meshes: true,
            })
            .add_systems(Startup, (setup_stage, setup_model).chain())
            .add_systems(Update, (start_clip, drive, finish_frames).chain())
            .add_observer(on_readback);
        if self.window {
            app.insert_resource(ClearColor(Color::srgb(0.12, 0.12, 0.14)))
                .add_systems(Startup, setup_preview.after(setup_model))
                .add_systems(Update, update_preview);
        }
    }
}

fn render_image(
    images: &mut Assets<Image>,
    size: (u32, u32),
    format: TextureFormat,
) -> Handle<Image> {
    let mut image = Image::new_target_texture(size.0, size.1, format, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    images.add(image)
}

fn setup_stage(mut commands: Commands, mut images: ResMut<Assets<Image>>, guest: Res<GuestRes>) {
    commands.spawn((
        DirectionalLight {
            illuminance: SUN_LUX,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::IDENTITY.looking_to(-SUN_TOWARD.normalize(), Vec3::Y),
    ));

    let size = guest.0.lock().expect("lock").opts.size;
    let colour = render_image(&mut images, size, TextureFormat::Rgba8UnormSrgb);
    let depth = render_image(&mut images, size, TextureFormat::R32Float);
    commands.spawn((
        GuestCamera,
        Camera3d {
            // The depth pass samples the depth buffer
            depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING)
                .into(),
            ..default()
        },
        Camera {
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            order: -1,
            ..default()
        },
        RenderTarget::from(colour.clone()),
        Msaa::Off,
        Tonemapping::default(),
        Exposure::default(),
        Projection::custom(HostProjection::default()),
        Transform::IDENTITY,
        LinearDepth {
            image: depth.clone(),
            near: 0.1,
            far: f32::INFINITY,
        },
    ));
    commands.insert_resource(Targets {
        size,
        colour,
        depth,
        settle: SETTLE_FRAMES,
    });
}

fn setup_model(mut bridge: Bridge, slot: Res<ModelSlot>, guest: Res<GuestRes>) {
    let Some(model) = slot.0.lock().expect("lock").take() else {
        return;
    };
    let spawned = bridge.spawn_animated(model);
    let scale = guest.0.lock().expect("lock").opts.scale;
    bridge.commands.entity(spawned.root).insert((
        GuestModel,
        Transform::from_scale(Vec3::splat(scale)),
        // Parked until the host's first camera says where it stands
        Visibility::Hidden,
    ));
    // The model is one object that is always in the shot; culling it by bounds that skinned meshes
    // do not keep would only make parts blink out
    for mesh in &spawned.meshes {
        bridge.commands.entity(*mesh).insert(NoFrustumCulling);
    }
}

fn start_clip(
    guest: Res<GuestRes>,
    mut players: Query<(&BridgeAnimations, &mut AnimationPlayer), Added<BridgeAnimations>>,
) {
    let Some(name) = guest.0.lock().expect("lock").opts.clip.clone() else {
        return;
    };
    for (clips, mut player) in &mut players {
        match clips.play(&mut player, &name) {
            Some(active) => {
                active.repeat();
            }
            None => warn!("clip {name:?} not found"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------------------------

type CameraQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Projection,
        &'static mut Camera,
        &'static mut RenderTarget,
        &'static mut LinearDepth,
    ),
    With<GuestCamera>,
>;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    guest: Res<GuestRes>,
    mut targets: ResMut<Targets>,
    mut images: ResMut<Assets<Image>>,
    mut inflight: ResMut<InFlight>,
    mut cameras: CameraQuery,
    mut models: Query<(&mut Transform, &mut Visibility), (With<GuestModel>, Without<GuestCamera>)>,
    shutdown: Res<ShutdownFlag>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let mut guard = guest.0.lock().expect("lock");
    let guest = &mut *guard;
    // The camera draws only in the frames a render is asked for
    if let Ok((_, _, mut camera, _, _)) = cameras.single_mut() {
        camera.is_active = false;
    }

    // Leaving: say bye, give it a moment to drain, then stop
    let asked = shutdown.0.load(Ordering::Relaxed)
        || guest
            .opts
            .seconds
            .is_some_and(|s| guest.started.elapsed().as_secs_f32() > s);
    if asked && guest.exit_at.is_none() {
        guest.log("shutting down");
        guest.link.disconnect("bridge-guest is done");
        guest.exit_at = Some(Instant::now() + Duration::from_millis(300));
    }

    for event in guest.link.poll() {
        match event {
            Event::Connected(peer) => guest.on_connected(&peer),
            Event::Lost { reason } => {
                guest.log(format!("LOST the host: {reason}"));
                guest.writer = None;
                guest.camera = None;
                guest.placement.reset();
                guest.frames_seen = 0;
            }
            Event::HandshakeFailed { reason } => guest.log(format!("handshake failed: {reason}")),
            Event::Camera { header, camera } => {
                let mut stats = guest.stats.lock().expect("lock");
                stats.cameras += 1;
                if guest.camera.is_some() {
                    stats.skipped += 1;
                }
                drop(stats);
                if let Some(msg) = guest.placement.on_camera(&camera) {
                    guest.log(msg);
                }
                guest.camera = Some(PendingCamera {
                    seq: header.seq,
                    camera,
                    received: Instant::now(),
                    host_time_us: header.time_us,
                });
            }
            Event::Collision(c) => {
                guest.log(format!(
                    "collision query {}: {} triangles within {} m of {:?}",
                    c.query,
                    c.triangles.len(),
                    c.radius,
                    c.center.map(|v| (v * 10.0).round() / 10.0)
                ));
                if let Some(msg) = guest.placement.on_collision(&c.triangles) {
                    guest.log(msg);
                }
            }
            Event::Focus { focus, .. } => guest.log(format!(
                "host says input focus is now with the {}",
                focus.as_str()
            )),
            _ => {}
        }
    }
    if guest.exit_at.is_some_and(|t| Instant::now() >= t) {
        let summary = guest.stats.lock().expect("lock").summary();
        guest.log(summary);
        exit.write(AppExit::Success);
        return;
    }

    // The model's place, in the app's world and on the wire
    if let Some(transform) = guest.placement.transform() {
        if let Ok((mut t, mut vis)) = models.single_mut() {
            *t = transform;
            *vis = Visibility::Inherited;
        }
    }
    if guest.link.is_connected() && guest.last_pose.elapsed() >= POSE_INTERVAL {
        if let Some(pose) = guest.placement.pose(ENTITY, now_us()) {
            guest.last_pose = Instant::now();
            let _ = guest.link.send_poses(&[pose]);
        }
    }

    guest.frames_seen = guest.frames_seen.saturating_add(1);
    request_render(
        guest,
        &mut targets,
        &mut images,
        &mut inflight,
        &mut cameras,
        &mut commands,
    );

    // Give up on renders that never came back
    inflight.jobs.retain(|_, job| {
        let stale = job.requested.elapsed() > GIVE_UP;
        if stale {
            for e in job.entities {
                commands.entity(e).despawn();
            }
        }
        !stale
    });

    let line = guest.stats.lock().expect("lock").tick();
    if let Some(line) = line {
        guest.log(line);
    }
}

fn request_render(
    guest: &mut Guest,
    targets: &mut Targets,
    images: &mut Assets<Image>,
    inflight: &mut InFlight,
    cameras: &mut CameraQuery,
    commands: &mut Commands,
) {
    if guest.camera.is_none()
        || guest.writer.is_none()
        || !guest.placement.is_placed()
        || guest.frames_seen < WARM_UP_FRAMES
        || inflight.jobs.len() >= MAX_IN_FLIGHT
    {
        return;
    }
    let Some(pending) = guest.camera.take() else {
        return;
    };
    let projection = match HostProjection::from_host(&pending.camera) {
        Ok(p) => p,
        Err(why) => {
            guest.warn_once(format!("camera ignored: {why}"));
            return;
        }
    };
    let Ok((mut transform, mut proj, mut camera, mut target, mut linear)) = cameras.single_mut()
    else {
        return;
    };

    let size = render_size(pending.camera.viewport, guest.opts.size);
    if size != targets.size {
        guest.log(format!(
            "rendering {}x{} for the host's {}x{} viewport",
            size.0, size.1, pending.camera.viewport[0], pending.camera.viewport[1]
        ));
        targets.size = size;
        targets.colour = render_image(images, size, TextureFormat::Rgba8UnormSrgb);
        targets.depth = render_image(images, size, TextureFormat::R32Float);
        targets.settle = SETTLE_FRAMES;
        *target = RenderTarget::from(targets.colour.clone());
        linear.image = targets.depth.clone();
    }
    if targets.settle > 0 {
        targets.settle -= 1;
        guest.camera = Some(pending);
        return;
    }

    *transform = transform_from_view(&pending.camera.view);
    *proj = Projection::custom(projection);
    linear.near = projection.near;
    linear.far = projection.far;
    camera.is_active = true;

    let id = inflight.next;
    inflight.next += 1;
    let colour = commands
        .spawn((
            ReadbackOnce::texture(targets.colour.clone()),
            Ticket {
                id,
                plane: Plane::Colour,
            },
        ))
        .id();
    let depth = commands
        .spawn((
            ReadbackOnce::texture(targets.depth.clone()),
            Ticket {
                id,
                plane: Plane::Depth,
            },
        ))
        .id();
    inflight.jobs.insert(
        id,
        Job {
            camera_seq: pending.seq,
            received: pending.received,
            host_time_us: pending.host_time_us,
            render_us: now_us(),
            requested: Instant::now(),
            far: projection.far,
            size,
            colour: None,
            depth: None,
            entities: [colour, depth],
        },
    );
}

fn on_readback(
    event: On<ReadbackComplete>,
    tickets: Query<&Ticket>,
    mut inflight: ResMut<InFlight>,
    mut commands: Commands,
) {
    let Ok(ticket) = tickets.get(event.entity) else {
        return;
    };
    commands.entity(event.entity).despawn();
    if let Some(job) = inflight.jobs.get_mut(&ticket.id) {
        let data = event.data.clone();
        match ticket.plane {
            Plane::Colour => job.colour = Some(data),
            Plane::Depth => job.depth = Some(data),
        }
    }
}

/// Writes every render whose two read-backs are in, and announces it
fn finish_frames(guest: Res<GuestRes>, mut inflight: ResMut<InFlight>) {
    let mut guard = guest.0.lock().expect("lock");
    let ready: Vec<u64> = inflight
        .jobs
        .iter()
        .filter(|(_, j)| j.colour.is_some() && j.depth.is_some())
        .map(|(id, _)| *id)
        .collect();
    // Oldest first, so frame numbers follow the cameras
    let mut ready = ready;
    ready.sort_unstable();
    for id in ready {
        let Some(job) = inflight.jobs.remove(&id) else {
            continue;
        };
        let (w, h) = (job.size.0 as usize, job.size.1 as usize);
        let (Some(colour), Some(depth)) = (job.colour.as_deref(), job.depth.as_deref()) else {
            continue;
        };
        let Some((rgba, metres)) = assemble(colour, depth, w, h, job.far) else {
            guard.warn_once("a read-back came back smaller than the image".into());
            continue;
        };
        let guest = &mut *guard;
        let Some(writer) = guest.writer.as_mut() else {
            continue;
        };
        match writer.write_depth(
            &rgba,
            Some(&metres),
            job.size.0,
            job.size.1,
            job.camera_seq,
            job.render_us,
        ) {
            Ok(ready) => {
                if guest.link.send_frame(&ready).is_ok() {
                    let mut stats = guest.stats.lock().expect("lock");
                    stats.frames += 1;
                    stats
                        .latency
                        .push(job.received.elapsed().as_secs_f32() * 1000.0);
                    let host_ms = now_us().saturating_sub(job.host_time_us) as f32 / 1000.0;
                    stats.host_latency.push(host_ms);
                }
            }
            Err(e) => guest.warn_once(format!("could not write a frame: {e}")),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// --window: show the colour image
// ---------------------------------------------------------------------------------------------

fn setup_preview(mut commands: Commands, targets: Res<Targets>) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 0,
            ..default()
        },
    ));
    commands.spawn((Preview, Sprite::from_image(targets.colour.clone())));
}

fn update_preview(
    targets: Res<Targets>,
    mut sprite: Query<(&mut Sprite, &mut Transform), With<Preview>>,
) {
    if !targets.is_changed() {
        return;
    }
    for (mut s, mut t) in &mut sprite {
        s.image = targets.colour.clone();
        let fit = (1280.0 / targets.size.0 as f32).min(720.0 / targets.size.1 as f32);
        t.scale = Vec3::splat(fit);
    }
}
