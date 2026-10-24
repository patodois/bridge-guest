//! A bare link host for measuring the guest: it sends a camera orbiting the origin at a chosen rate
//! (or as fast as the guest answers), reads every frame and reports how many arrive per second, how
//! old they are, and what is in them (alpha values, depth range). It composites nothing.
//!
//!   cargo run --release --example probe_host -- [--port 47800] [--seconds 10] [--rate 60]
//!       [--viewport 1280x720] [--dump frame.ppm] [--eye-distance 6]
//!
//! `--rate 0` sends a new camera whenever a frame came back, so the guest runs flat out. The camera
//! looks at (0, 1, 0) from `--eye-distance` metres, 1.6 m up, and goes round once in 10 s.

#![allow(unknown_lints, clippy::chunks_exact_to_as_chunks)]

use std::time::{Duration, Instant};

use rewrite_bridge::link::*;

fn arg<T: std::str::FromStr>(name: &str) -> Option<T> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1)?.parse().ok()
}

fn camera_at(t: f32, distance: f32, viewport: [u32; 2]) -> Camera {
    let a = t * std::f32::consts::TAU / 10.0;
    Camera::look_at(
        [distance * a.sin(), 1.6, distance * a.cos()],
        [0.0, 1.0, 0.0],
        1.0,
        0.1,
        f32::INFINITY,
        viewport,
    )
    .with_depth_range(DepthRange::Reversed)
}

fn write_ppm(path: &str, w: usize, h: usize, rgba: &[u8]) -> std::io::Result<()> {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for px in rgba.chunks_exact(4) {
        // Over a mid grey, so empty pixels show
        let a = u16::from(px[3]);
        out.extend(
            px[..3]
                .iter()
                .map(|&c| ((u16::from(c) * a + 90 * (255 - a)) / 255) as u8),
        );
    }
    std::fs::write(path, out)
}

fn main() -> std::io::Result<()> {
    let port: u16 = arg("--port").unwrap_or(DEFAULT_CONTROL_PORT);
    let seconds: f32 = arg("--seconds").unwrap_or(10.0);
    let rate: f32 = arg("--rate").unwrap_or(60.0);
    let distance: f32 = arg("--eye-distance").unwrap_or(6.0);
    let viewport: [u32; 2] = arg::<String>("--viewport")
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1280, 720]);
    let dump: Option<String> = arg("--dump");

    let start = Instant::now();
    let mut link = Link::host(LinkConfig {
        control_port: port,
        ..LinkConfig::new("probe_host")
    })?;
    println!("listening on {}", link.control_addr().unwrap());
    let mut reader: Option<FrameReader> = None;
    let (mut rgba, mut depth) = (Vec::new(), Vec::new());
    let mut next_send = Instant::now();
    let mut waiting = false;
    let (mut sent, mut got) = (0u32, 0u32);
    let (mut total_sent, mut total_got) = (0u64, 0u64);
    let mut ages = Vec::new();
    let mut window = Instant::now();
    let (mut partial_alpha, mut covered, mut pixels) = (0u64, 0u64, 0u64);
    let (mut dmin, mut dmax) = (f32::INFINITY, 0.0f32);
    let mut size = (0, 0);
    let mut began_frames: Option<Instant> = None;

    while start.elapsed().as_secs_f32() < seconds {
        for event in link.poll() {
            match event {
                Event::Connected(p) => println!("connected: {}", p.game),
                Event::Lost { reason } => {
                    println!("lost: {reason}");
                    reader = None;
                }
                Event::FrameChannel(c) => reader = FrameReader::open(c.path.as_ref()).ok(),
                Event::Frame { frame, .. } => {
                    waiting = false;
                    if let Some(r) = &reader {
                        if let Ok(d) = r.read_depth(&frame, &mut rgba, &mut depth) {
                            began_frames.get_or_insert_with(Instant::now);
                            got += 1;
                            total_got += 1;
                            ages.push(now_us().saturating_sub(d.time_us) as f32 / 1000.0);
                            size = (d.width as usize, d.height as usize);
                            for (px, &t) in rgba.chunks_exact(4).zip(&depth) {
                                pixels += 1;
                                if px[3] != 0 {
                                    covered += 1;
                                    if px[3] != 255 {
                                        partial_alpha += 1;
                                    }
                                    dmin = dmin.min(t);
                                    dmax = dmax.max(t);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if link.is_connected() {
            let due = if rate > 0.0 {
                Instant::now() >= next_send
            } else {
                !waiting
            };
            if due {
                if rate > 0.0 {
                    next_send += Duration::from_secs_f32(1.0 / rate);
                }
                if link
                    .send_camera(&camera_at(
                        start.elapsed().as_secs_f32(),
                        distance,
                        viewport,
                    ))
                    .is_ok()
                {
                    sent += 1;
                    total_sent += 1;
                    waiting = true;
                }
            }
            // A frame that never comes (dropped datagram) must not stall --rate 0
            if waiting && rate <= 0.0 && Instant::now() > next_send + Duration::from_millis(200) {
                waiting = false;
                next_send = Instant::now();
            }
            if rate <= 0.0 && !waiting {
                next_send = Instant::now();
            }
        }
        if window.elapsed() >= Duration::from_secs(1) {
            let secs = window.elapsed().as_secs_f32();
            let mean = ages.iter().sum::<f32>() / ages.len().max(1) as f32;
            println!(
                "cameras {:6.1}/s, frames {:6.1}/s, frame age at the host {:5.1} ms, {}x{}",
                sent as f32 / secs,
                got as f32 / secs,
                mean,
                size.0,
                size.1
            );
            (sent, got) = (0, 0);
            ages.clear();
            window = Instant::now();
        }
        std::thread::sleep(Duration::from_micros(if rate <= 0.0 { 100 } else { 1000 }));
    }
    let secs = began_frames
        .map_or(1.0, |t| t.elapsed().as_secs_f32())
        .max(1e-3);
    println!(
        "total: {total_sent} cameras sent, {total_got} frames read ({:.1} fps over {secs:.1} s of frames)",
        total_got as f32 / secs
    );
    println!(
        "pixels seen: {covered} covered of {pixels}; with alpha other than 0 or 255: {partial_alpha}; depth {dmin:.3} to {dmax:.3} m"
    );
    if let Some(path) = dump {
        write_ppm(&path, size.0, size.1, &rgba)?;
        println!("wrote {path}");
    }
    link.disconnect("probe done");
    for _ in 0..40 {
        link.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
