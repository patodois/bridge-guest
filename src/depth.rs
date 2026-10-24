//! Linear depth out of the main pass, on the GPU.
//!
//! The camera draws as usual into its colour image. Its depth buffer (reversed-Z, see `camera.rs`)
//! is then read by one fullscreen pass that writes `t = near / d`, the distance along the view axis
//! in metres, into an `R32Float` image. Both images are read back with Bevy's `Readback`.
//!
//! The pass is a system of the render world's `Core3d` schedule, after the main passes. The depth
//! texture is Bevy's own (`Depth32Float`, no MSAA); the camera asks for it to be bindable.

use std::num::NonZeroU64;

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{texture_depth_2d, uniform_buffer_sized};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BufferInitDescriptor,
    BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState, Operations,
    PipelineCache, RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor,
    ShaderStages, TextureFormat,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewDepthStencilTexture;
use bevy::render::{RenderApp, RenderStartup};

/// On the camera: where to write its linear depth.
#[derive(Component, ExtractComponent, Clone)]
#[extract_app(RenderApp)]
pub struct LinearDepth {
    /// An `R32Float` render-target image of the camera's size
    pub image: Handle<Image>,
    /// The projection's near plane, metres
    pub near: f32,
    /// The host's far plane, metres, or infinity
    pub far: f32,
}

pub struct LinearDepthPlugin;

impl Plugin for LinearDepthPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "linear_depth.wgsl");
        app.add_plugins(ExtractComponentPlugin::<LinearDepth>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(Core3d, linear_depth_pass.in_set(Core3dSystems::PostProcess));
    }
}

#[derive(Resource)]
struct DepthPipeline {
    layout: BindGroupLayoutDescriptor,
    id: CachedRenderPipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "linear_depth_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_depth_2d(),
                uniform_buffer_sized(false, NonZeroU64::new(16)),
            ),
        ),
    );
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("linear_depth_pipeline".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(asset_server.as_ref(), "linear_depth.wgsl"),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::R32Float,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(DepthPipeline { layout, id });
}

fn linear_depth_pass(
    view: ViewQuery<(&ViewDepthStencilTexture, &LinearDepth)>,
    pipeline: Option<Res<DepthPipeline>>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (depth, target) = view.into_inner();
    let Some(pipeline) = pipeline else { return };
    let Some(render_pipeline) = cache.get_render_pipeline(pipeline.id) else {
        return;
    };
    let Some(out) = images.get(&target.image) else {
        return;
    };

    let mut params = [0u8; 16];
    params[0..4].copy_from_slice(&target.near.to_le_bytes());
    params[4..8].copy_from_slice(&target.far.to_le_bytes());
    let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("linear_depth_params"),
        contents: &params,
        usage: BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(
        "linear_depth_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            &depth.attachment.texture.default_view,
            uniform.as_entire_binding(),
        )),
    );

    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("linear_depth_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &out.texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}
