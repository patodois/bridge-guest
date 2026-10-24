//! bridge-guest: any rewrite-bridge provider's model, inside any game that hosts the passthrough
//! link. See README.md.

mod bounds;
mod camera;
mod cli;
mod depth;
mod frames;
mod placement;
mod providers;
mod render;
mod selftest;
mod stats;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::{default, App, DefaultPlugins, PluginGroup, Window, WindowPlugin};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use clap::Parser;
use rewrite_bridge::link::{Link, LinkConfig};
use rewrite_bridge::AlphaMode;
use rewrite_bridge_bevy::RewriteBridgePlugin;

use crate::cli::Opts;
use crate::placement::{Placement, PlacementOptions};
use crate::render::{Guest, GuestPlugin, GuestRes, ModelSlot, ShutdownFlag};
use crate::stats::Stats;

fn main() -> Result<()> {
    let opts = Opts::parse();
    let provider = providers::open(&opts)?;

    if let Some(filter) = &opts.list {
        let mut items = provider.list(filter).map_err(|e| anyhow::anyhow!("{e}"))?;
        items.sort_by(|a, b| a.id.cmp(&b.id));
        for item in &items {
            println!("{}\t{:?}\t{}", item.id, item.kind, item.name);
        }
        eprintln!("{} assets", items.len());
        return Ok(());
    }
    let id = opts.id.clone().expect("clap requires --id without --list");

    let began = std::time::Instant::now();
    let mut model = provider
        .load_animated(&id)
        .map_err(|e| anyhow::anyhow!("{} could not load {id:?}: {e}", provider.game()))?;
    model
        .validate()
        .map_err(|e| anyhow::anyhow!("{} gave an invalid model: {e}", provider.game()))?;
    println!(
        "loaded {:?} from {} in {:.1}s: {} meshes, {} nodes, {} clips",
        model.model.name,
        provider.game(),
        began.elapsed().as_secs_f32(),
        model.model.meshes.len(),
        model.model.nodes.len(),
        model.animations.len()
    );
    if opts.list_clips {
        for a in &model.animations {
            println!("{}\t{:.2}s", a.name, a.duration_s);
        }
        return Ok(());
    }
    if let Some(clip) = &opts.clip {
        if model.animation(clip).is_none() {
            let names: Vec<&str> = model.animations.iter().map(|a| a.name.as_str()).collect();
            let shown = names
                .iter()
                .take(12)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "the model has no clip {clip:?}; it has {} clip(s){}{}{} (--list-clips prints them all)",
                names.len(),
                if names.is_empty() { "" } else { ": " },
                shown,
                if names.len() > 12 { ", ..." } else { "" }
            );
        }
    }
    // One surface per pixel: a translucent material becomes a cut-out, so it has a depth to send
    for m in &mut model.model.materials {
        if matches!(m.alpha, AlphaMode::Blend) {
            m.alpha = AlphaMode::Mask(0.5);
        }
    }
    let rest = bounds::rest_bounds(&model.model).context("the model has no vertices")?;
    println!(
        "model bounds at rest: {:?} to {:?} (x {:.2} m, y {:.2} m, z {:.2} m)",
        rest.min.to_array(),
        rest.max.to_array(),
        rest.max.x - rest.min.x,
        rest.max.y - rest.min.y,
        rest.max.z - rest.min.z
    );

    let link = Link::guest(LinkConfig {
        control_port: opts.port,
        ..LinkConfig::new(format!("bridge-guest ({})", provider.game()))
    })
    .with_context(|| "could not open the link sockets")?;
    println!("connecting to 127.0.0.1:{}", opts.port);

    let placement = Placement::new(
        PlacementOptions {
            in_front: opts.in_front,
            distance: opts.distance,
            yaw: opts.yaw.map(f32::to_radians),
            scale: opts.scale,
        },
        rest,
    );
    let stats = Arc::new(Mutex::new(Stats::new()));
    let shutdown = Arc::new(AtomicBool::new(false));
    {
        let flag = shutdown.clone();
        ctrlc::set_handler(move || flag.store(true, std::sync::atomic::Ordering::Relaxed))
            .context("could not install the Ctrl-C handler")?;
    }
    let guest = Guest::new(
        link,
        opts.clone(),
        placement,
        stats,
        provider.game().to_string(),
        model.model.name.clone(),
    );
    // The reader is done; free what it holds before Bevy takes its share of memory
    drop(provider);

    let mut app = App::new();
    if opts.window {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "bridge-guest".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            ..default()
        }));
    } else {
        // No window at all: covered windows render black on macOS, and none is needed
        app.add_plugins((
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>()
                .set(bevy::render::RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                }),
            ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)),
        ));
    }
    app.add_plugins((
        RewriteBridgePlugin::default(),
        depth::LinearDepthPlugin,
        GuestPlugin {
            window: opts.window,
        },
    ))
    .insert_resource(GuestRes(Mutex::new(guest)))
    .insert_resource(ModelSlot(Mutex::new(Some(model))))
    .insert_resource(ShutdownFlag(shutdown));
    app.run();
    Ok(())
}
