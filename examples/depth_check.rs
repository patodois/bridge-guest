//! Measures the depth bridge-guest sends against a plane whose depth is known exactly.
//!
//! This is a link host. For each case it starts `bridge-guest --provider selftest` (a big flat wall
//! or ground, no game data), sends it a camera and reads the depth plane of the frame that answers
//! it. The truth is the linear view depth of the plane along each pixel's ray, in f64.
//!
//!   cargo build --release && cargo run --release --example depth_check -- \
//!       [--guest target/release/bridge-guest] [--port 47811]
//!
//! Cases: a wall facing the camera at 1, 5, 20 and 100 m (the depth is the same at every pixel, and
//! is not the distance to the eye), and a ground seen from 1.7 m and 50 m up (the depth runs from
//! about 2 m to the horizon). Each with the three depth conventions of `Camera.depth_range`, and with
//! a finite and an infinite far plane.

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rewrite_bridge::link::*;

fn arg<T: std::str::FromStr>(name: &str) -> Option<T> {
    let args: Vec<String> = std::env::args().collect();
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1)?.parse().ok()
}

const VIEWPORT: [u32; 2] = [640, 360];
const NEAR: f32 = 0.1;
const FOV: f32 = 1.0;

struct Case {
    label: String,
    asset: &'static str,
    camera: Camera,
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    let ranges = [
        ("range0", DepthRange::NegOneToOne),
        ("range1", DepthRange::ZeroToOne),
        ("range2", DepthRange::Reversed),
    ];
    let fars = [("far1000", 1000.0f32), ("farinf", f32::INFINITY)];
    for (rname, range) in ranges {
        for (fname, far) in fars {
            for d in [1.0f32, 5.0, 20.0, 100.0] {
                // The wall is at z = 0 and spans y 0..4000; look straight at it from y = 200
                let camera =
                    Camera::look_at([0.0, 200.0, d], [0.0, 200.0, 0.0], FOV, NEAR, far, VIEWPORT)
                        .with_depth_range(range);
                out.push(Case {
                    label: format!("wall {d:>5} m  {rname} {fname}"),
                    asset: "wall",
                    camera,
                });
            }
            for h in [1.7f32, 50.0] {
                let camera = Camera::look_at(
                    [0.0, h, 0.0],
                    [0.0, 0.0, -5.0 * h / 1.7],
                    FOV,
                    NEAR,
                    far,
                    VIEWPORT,
                )
                .with_depth_range(range);
                out.push(Case {
                    label: format!("ground from {h:>4} m  {rname} {fname}"),
                    asset: "ground",
                    camera,
                });
            }
        }
    }
    out
}

/// Linear view depth of the plane `y = 0` (ground) or `z = 0` (wall) along the pixel's ray, or
/// `None` when the ray misses it (or only meets it behind the camera)
fn truth(camera: &Camera, asset: &str, x: usize, y: usize, w: usize, h: usize) -> Option<f64> {
    let ndc_x = (x as f64 + 0.5) / w as f64 * 2.0 - 1.0;
    let ndc_y = 1.0 - (y as f64 + 0.5) / h as f64 * 2.0;
    // View-space ray with z = -1, so the parameter along it is the linear depth
    let rv = [
        ndc_x / f64::from(camera.proj[0]),
        ndc_y / f64::from(camera.proj[5]),
        -1.0,
    ];
    // World ray: the transpose of the view's rotation applied to it
    let v = &camera.view;
    let rw: [f64; 3] =
        std::array::from_fn(|c| (0..3).map(|r| f64::from(v[c * 4 + r]) * rv[r]).sum());
    let eye = camera.eye().map(f64::from);
    let s = match asset {
        "ground" => -eye[1] / rw[1],
        _ => -eye[2] / rw[2],
    };
    (s > 0.0 && s.is_finite()).then_some(s)
}

#[derive(Default)]
struct Errors {
    n: usize,
    sum_abs: f64,
    max_abs: f64,
    max_rel: f64,
    /// The same over the pixels within 100 m, where a game's models are
    near_max_abs: f64,
    near_max_rel: f64,
    rels: Vec<f64>,
    /// Pixels the truth says are covered (and within far) that came back empty
    missing: usize,
    /// Pixels that came back with depth where the truth says nothing is
    extra: usize,
}

fn compare(case: &Case, depth: &[f32], rgba: &[u8], w: usize, h: usize) -> Errors {
    let far = f64::from(case.camera.far);
    let mut e = Errors::default();
    for y in 0..h {
        for x in 0..w {
            let got = f64::from(depth[y * w + x]);
            let alpha = rgba[(y * w + x) * 4 + 3];
            let t = truth(&case.camera, case.asset, x, y, w, h);
            // The planes end at 2000 m from their middle: outside that nothing is drawn
            let expect = t.filter(|&t| t <= far && t >= f64::from(NEAR) && t < 1900.0);
            match (expect, got.is_finite() && alpha != 0) {
                (Some(t), true) => {
                    let abs = (got - t).abs();
                    let rel = abs / t;
                    e.n += 1;
                    e.sum_abs += abs;
                    e.max_abs = e.max_abs.max(abs);
                    e.max_rel = e.max_rel.max(rel);
                    e.rels.push(rel);
                    if t <= 100.0 {
                        e.near_max_abs = e.near_max_abs.max(abs);
                        e.near_max_rel = e.near_max_rel.max(rel);
                    }
                }
                (Some(t), false) => {
                    // The plane's own edge and the ray at grazing angles are not errors
                    if t < 1500.0 {
                        e.missing += 1;
                    }
                }
                (None, true) => {
                    if t.is_none_or(|t| t > far + 1.0 || t < 1.0) {
                        e.extra += 1;
                    }
                }
                (None, false) => {}
            }
        }
    }
    e
}

fn start_guest(path: &str, asset: &str, port: u16) -> std::io::Result<Child> {
    Command::new(path)
        .args([
            "--provider",
            "selftest",
            "--id",
            asset,
            "--port",
            &port.to_string(),
        ])
        .args(["--size", "640x360"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

fn main() -> std::io::Result<()> {
    let guest_path: String = arg("--guest").unwrap_or_else(|| "target/release/bridge-guest".into());
    let port: u16 = arg("--port").unwrap_or(47811);
    let all = cases();
    println!(
        "{:<44} {:>8} {:>11} {:>11} {:>10} {:>10} {:>13} {:>10} {:>7} {:>6}",
        "case",
        "pixels",
        "max abs m",
        "mean abs m",
        "max rel",
        "p99 rel",
        "<=100m abs m",
        "<=100m rel",
        "missing",
        "extra"
    );
    let mut worst_rel: f64 = 0.0;
    let mut failures = 0;
    for asset in ["wall", "ground"] {
        let mut link = Link::host(LinkConfig {
            control_port: port,
            ..LinkConfig::new("depth_check")
        })?;
        let mut child = start_guest(&guest_path, asset, port)?;
        let mut reader: Option<FrameReader> = None;
        let (mut rgba, mut depth) = (Vec::new(), Vec::new());
        let deadline_total = Instant::now() + Duration::from_secs(300);
        for case in all.iter().filter(|c| c.asset == asset) {
            let mut seqs: HashMap<u32, ()> = HashMap::new();
            let mut last_send = Instant::now() - Duration::from_secs(1);
            let began = Instant::now();
            let mut result: Option<Errors> = None;
            while result.is_none() {
                if began.elapsed() > Duration::from_secs(40) || Instant::now() > deadline_total {
                    break;
                }
                for event in link.poll() {
                    match event {
                        Event::FrameChannel(c) => reader = FrameReader::open(c.path.as_ref()).ok(),
                        Event::Frame { frame, .. } => {
                            if let (Some(r), true) = (&reader, seqs.contains_key(&frame.camera_seq))
                            {
                                if let Ok(d) = r.read_depth(&frame, &mut rgba, &mut depth) {
                                    let (w, h) = (d.width as usize, d.height as usize);
                                    result = Some(compare(case, &depth, &rgba, w, h));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if link.is_connected() && last_send.elapsed() > Duration::from_millis(30) {
                    last_send = Instant::now();
                    if let Ok(seq) = link.send_camera(&case.camera) {
                        seqs.insert(seq, ());
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            match result {
                Some(mut e) => {
                    e.rels.sort_by(f64::total_cmp);
                    let p99 = e.rels.get(e.rels.len() * 99 / 100).copied().unwrap_or(0.0);
                    worst_rel = worst_rel.max(e.max_rel);
                    println!(
                        "{:<44} {:>8} {:>11.6} {:>11.8} {:>10.2e} {:>10.2e} {:>13.6} {:>10.2e} {:>7} {:>6}",
                        case.label,
                        e.n,
                        e.max_abs,
                        e.sum_abs / e.n.max(1) as f64,
                        e.max_rel,
                        p99,
                        e.near_max_abs,
                        e.near_max_rel,
                        e.missing,
                        e.extra
                    );
                    if e.n == 0 {
                        failures += 1;
                    }
                }
                None => {
                    failures += 1;
                    println!("{:<44} no frame came back", case.label);
                }
            }
        }
        link.disconnect("done");
        for _ in 0..50 {
            link.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = child.kill();
        let _ = child.wait();
        drop(link);
    }
    println!("worst relative error over every compared pixel: {worst_rel:.3e}");
    if failures > 0 {
        eprintln!("{failures} case(s) failed");
        std::process::exit(1);
    }
    Ok(())
}
