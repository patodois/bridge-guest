# bridge-guest

Puts a model from any [rewrite-bridge](https://github.com/patodois/rewrite-bridge) provider (War Thunder, Warframe, Hytale,
Nocturne) into any game that hosts the passthrough link (`docs/CONTRACT.md` of rewrite-bridge, v1 revision 4).

It connects to the host as the link's guest and spawns one entity. For every `Camera` the host sends, it renders the model on the
GPU (Bevy 0.20, Metal/Vulkan/DX12) with exactly that camera, reads back colour and linear depth in metres, writes both into the
pixel file and answers with a `Frame` carrying the camera's `seq`. The host reprojects the frame and depth-tests it against its own
depth buffer, so the model stands in the host's world and is hidden behind the host's walls. `fake_guest` does the same with a
CPU-rasterised cube; this is the real one.

**It ships no game data.** The program reads the files of your own copy of the game that you point it at. Nothing is bundled,
cached or written next to it (the pixel file lives in the temp directory and is deleted when the session ends).

## Run it

```sh
cargo build --release          # all four providers; --no-default-features --features hytale for one

# the host first (a game with the link, or rewrite-bridge's fake_host), then:
bridge-guest --provider hytale --id "Server/Models/Intelligent/Trork/Trork.json" --clip Walk --in-front
bridge-guest --provider wt --game /path/to/WarThunder/game --id "tanks/usa_m4a1_76w_sherman.grp:m4a1_76w_sherman" --in-front
bridge-guest --provider wf --game /path/to/Cache.Windows --id "/Lotus/Characters/Tenno/Excalibur/Excalibur_skel.fbx?animations=/Lotus/Animations/Tenno/Movement/MeleeWeapon/Sword/Jog*" --clip /Lotus/Animations/Tenno/Movement/MeleeWeapon/Sword/JogForward_anim.fbx --in-front
```

| Option | |
|---|---|
| `--provider wt\|wf\|hytale\|nocturne` | which game's assets |
| `--game PATH` | War Thunder: the game folder. Warframe: `Cache.Windows`. Hytale: `Assets.zip` (optional, the launcher's install by default). Nocturne: the folder holding `smt3hd_Data` |
| `--id ID` | the id the provider lists (`--list FILTER` prints them) |
| `--clip NAME` | loop this animation clip (`--list-clips` prints them). A wrong name lists the right ones |
| `--port 47800` | the host's control port |
| `--size 640x360` | the largest image sent. The host's aspect ratio is kept inside it, so pixels stay square |
| `--in-front` | stand `--distance` metres (default 4, more for a big model) ahead of the first camera, facing it, on the host's ground (from its `Collision` snapshot). Without it the model stands at the world origin |
| `--scale 1.0`, `--yaw DEG`, `--distance M` | size, heading, and distance for `--in-front` |
| `--max-texture 1024` | Warframe: largest baked texture side (0 = full size) |
| `--window` | also show the colour image in a window. By default there is no window at all |
| `--seconds N` | quit after N seconds (Ctrl-C and SIGTERM also say `bye` and clean up) |

Once a second it prints cameras and frames per second and the latency from "camera received" to "frame sent", and a summary at the end.

Ids that work (verified on the author's installs): Hytale `Server/Models/Intelligent/Trork/Trork.json` (clips `Walk`, `Run`, `Idle`, ...),
War Thunder `tanks/usa_m4a1_76w_sherman.grp:m4a1_76w_sherman`, Warframe `/Lotus/Characters/Tenno/Excalibur/Excalibur_skel.fbx`
(clips come from `?animations=<filter>`; pass the same filters as the `bridge-testbed` config). Nocturne builds and is wired the same
way but was not run (its files are not on the development machine). Any id of a provider's `list` that `load_animated` accepts should work.

## How it works

```text
host Camera ─> camera Transform + HostProjection ─> main pass ─> colour image (Rgba8UnormSrgb) ─┐
                                              └─> depth buffer ─> linear-depth pass ─> R32Float ─┴─> Readback x2
host <─ Frame <─ FrameWriter (pixel file: colour + depth) <─ assemble() <─ ReadbackComplete  <─────┘
```

- **Camera.** The view's inverse is the Bevy camera's `Transform` (both are right-handed, +Y up, looking down -Z, so nothing is
  converted). The projection keeps the host's x, y and w rows, so field of view, aspect and any off-centre shift are the host's, and
  replaces its clip-depth row with Bevy's: reversed, infinite far, `z_clip = near`, which makes the depth buffer hold `near / t`
  whatever `depth_range` the host uses (0, 1 or 2). The host's far plane is applied when the depth is read back. Only perspective
  projections are accepted; others are refused with a warning.
- **Depth.** One fullscreen pass in the render world (after the main passes) reads Bevy's own `Depth32Float` buffer and writes
  `t = near / d`, the distance along the view axis in metres (not the distance to the eye), into an `R32Float` image. No depth is
  guessed on the CPU. Pixels with no surface, or beyond the host's far plane, get alpha 0 and depth `+inf`, so colour and depth agree.
- **Readback.** Bevy's asynchronous `Readback` on both images, one request per camera, at most two renders in flight, the newest
  camera wins when both are busy. The app never waits for the GPU; the frame is written when both read-backs are in, a frame or
  two later, and announced with the `camera_seq` it was rendered for.
- **Pose.** The root's pose is sent about every 16 ms (teleport flag on the first). The model is lifted so its lowest point touches
  the host's ground; with `--in-front` the footprint is centred on the spot.
- **Lighting.** A fixed neutral stage (one sun, 10000 lux, from above and in front; ambient 400 cd/m2; no shadows), the same values as
  `bridge-testbed`. It is not the host's lighting, which the link does not carry.

## Measured here

Apple M5, macOS, Metal, release build, the Hytale Trork walking (30 meshes, 70 nodes):

| Host | Image | Frames/s | Camera received to frame sent |
|---|---|---|---|
| Minecraft 26.3 dev client, 60 cameras/s | 640x360 | 60 (every camera answered) | mean 4.4 ms, p95 4.9 ms |
| rewrite-bridge `fake_host`, about 50 cameras/s | 320x240 | 50 | mean 4.0 ms, p95 4.8 ms |
| `probe_host` at 60 cameras/s | 1280x720 | 58 | mean 6.1 ms, p95 6.2 ms |
| `probe_host` flat out (a camera per frame) | 640x360 | 417 | mean 3.7 ms, p95 4.1 ms |
| `probe_host` flat out | 1280x720 | 237 | mean 5.4 ms, p95 6.0 ms |

The host's own clock (its camera timestamp to our frame sent) reads about 0.7 ms more. Resident memory: about 0.7 GB for Excalibur
(textures at 512), 1.1 GB for the Sherman.

Depth against a plane of known depth (`examples/depth_check.rs`: the `selftest` provider, 3 depth conventions x finite and infinite far):

| Case | Pixels | Max error | Max relative error |
|---|---|---|---|
| wall facing the camera, 1 m | 230400 | 0 | 0 |
| wall, 5 m | 230400 | 0 | 0 |
| wall, 20 m | 230400 | 0 | 0 |
| wall, 100 m | 230400 | 0.0000076 m | 7.6e-8 (one f32 step) |
| ground seen from 1.7 m up, pixels within 100 m | | 7.1 mm | 7.9e-5 |
| ground seen from 50 m up, pixels within 100 m | | 0.36 mm | 4.2e-6 |
| ground from 1.7 m, whole view out to 1.9 km, far infinite | 186880 | 1.8 m (at 1.1 km, grazing) | 1.6e-3 (p99 3.3e-4) |

The wall tests the conversion itself (the same value at every pixel, so no rasterisation error): it is exact to float precision at
every distance and in all six camera conventions. The ground errors come from where the rasteriser samples a surface that runs
away at a grazing angle, not from the conversion.

## Tests

```sh
cargo test --release                                   # camera maths, placement, frame assembly, ...
cargo run --release --example depth_check              # the table above (needs the release binary)
cargo run --release --example probe_host -- --rate 0   # frame rate and latency against the guest
```

Against the real hosts: rewrite-bridge's `fake_host` (`--dump` writes the composited frames; start the guest with
`--size 320x240`, since `fake_host` only presents frames of its own 320x240) and a Minecraft 26.3 Fabric dev client running
`autotest/host_wall.txt` of minecraft-link with this program in place of `fake_guest`: the model stands on the flat world 4 m ahead,
facing the player, and a stone wall hides the half of it behind the wall.

## Limits

- One surface per pixel (that is what a depth plane holds): translucent materials become cut-outs (alpha 0.5), glass is not glass.
- No anti-aliasing: alpha is 0 or 1, so edges are jagged at low resolution. Render larger (`--size`) for smoother edges.
- Perspective hosts only. No shadows onto the host's world, no host lighting, no reflections of it.
- A clip with root motion walks the model away from the pose the host knows, and loops back. Use in-place clips.
- One entity, no scenes, no sockets or attachments, no input handling (the link's input focus is ignored).
- `--window` is for looking: it runs at the display's rate and adds latency.
- The model's bounds for standing on the ground are those of the rest pose.

## Dependencies

`rewrite-bridge` 0.2.1 (with its `link` feature) and `rewrite-bridge-bevy` 0.2.1 from crates.io, Bevy 0.20, and the providers
(`wt-unpack`, `wf-unpack`, `hytale-unpack`, `nocturne-unpack`) by git at pinned revisions. The providers take `rewrite-bridge` 0.2.1
from crates.io too, so `Cargo.lock` holds exactly one copy of it (`grep -c 'name = "rewrite-bridge"$' Cargo.lock` prints 1).

License: MIT OR Apache-2.0.
