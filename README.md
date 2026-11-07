# bridge-guest

Puts a model from any [rewrite-bridge](https://github.com/patodois/rewrite-bridge) provider (War Thunder, Warframe, Hytale,
Nocturne, Minecraft) into any game that hosts the passthrough link (`docs/CONTRACT.md` of rewrite-bridge, v1). It is built on rewrite-bridge 0.4.0, whose link implements revision 7 of the contract, so it can connect to a host that requires a token (`--endpoint-file`, `--token-file`, below).

**Adding a game as a host?** Read the [guide to adding a game](https://github.com/patodois/rewrite-bridge/blob/main/docs/INTEGRATING.md). This program is a real guest: start it against your host with `--in-front` to see a provider's model stand in your world. If your host requires a token (contract section 6.2), pass its endpoint file with `--endpoint-file`; see "A host that requires a token" below.

It connects to the host as the link's guest and spawns one entity. For every `Camera` the host sends, it renders the model on the
GPU (Bevy 0.20, Metal/Vulkan/DX12) with exactly that camera, reads back colour and linear depth in metres, writes both into the
pixel file and answers with a `Frame` carrying the camera's `seq`. The host reprojects the frame and depth-tests it against its own
depth buffer, so the model stands in the host's world and is hidden behind the host's walls. `fake_guest` does the same with a
CPU-rasterised cube; this is the real one.

**It ships no game data.** The program reads the files of your own copy of the game that you point it at. Nothing is bundled,
cached or written next to it (the pixel file lives in the temp directory and is deleted when the session ends).

## Run it

```sh
cargo build --release          # all five providers; --no-default-features --features hytale for one

# the host first (a game with the link, or rewrite-bridge's fake_host), then:
bridge-guest --provider hytale --id "Server/Models/Intelligent/Trork/Trork.json" --clip Walk --in-front
bridge-guest --provider wt --game /path/to/WarThunder/game --id "tanks/usa_m4a1_76w_sherman.grp:m4a1_76w_sherman" --in-front
bridge-guest --provider wf --game /path/to/Cache.Windows --id "/Lotus/Characters/Tenno/Excalibur/Excalibur_skel.fbx?animations=/Lotus/Animations/Tenno/Movement/MeleeWeapon/Sword/Jog*" --clip /Lotus/Animations/Tenno/Movement/MeleeWeapon/Sword/JogForward_anim.fbx --in-front
```

| Option | |
|---|---|
| `--provider wt\|wf\|hytale\|nocturne\|mc` | which game's assets |
| `--game PATH` | War Thunder: the game folder. Warframe: `Cache.Windows`. Hytale: `Assets.zip` (optional, the launcher's install by default). Nocturne: the folder holding `smt3hd_Data`. Minecraft: a Java Edition client jar (optional, the newest one a launcher installed by default; 1.12.2 to 26.3) |
| `--id ID` | the id the provider lists (`--list FILTER` prints them) |
| `--clip NAME` | loop this animation clip (`--list-clips` prints them). A wrong name lists the right ones |
| `--port 47800` | the host's control port |
| `--endpoint-file FILE` | the host's endpoint file (port and token, written by a host that requires a token; contract section 6.2). Read again before every attempt, so a restarted host is found. Replaces `--port` and `--token-file` |
| `--token-file FILE` | with `--port`: send the token on the first line of this file, for a host whose token is fixed rather than generated |
| `--size 640x360` | the largest image sent. The host's aspect ratio is kept inside it, so pixels stay square |
| `--in-front` | stand `--distance` metres (default 4, more for a big model) ahead of the first camera, facing it, on the host's ground (from its `Collision` snapshot). Without it the model stands at the world origin |
| `--scale 1.0`, `--yaw DEG`, `--distance M` | size, heading, and distance for `--in-front` |
| `--max-texture 1024` | Warframe: largest baked texture side (0 = full size) |
| `--window` | also show the colour image in a window. By default there is no window at all |
| `--seconds N` | quit after N seconds (Ctrl-C and SIGTERM also say `bye` and clean up) |

### A host that requires a token

A host that sets a token (or asks the library to make one) refuses a guest whose `hello` does not carry it, with `bye` `unauthorized`. The host
writes the port and the token to an owner-only endpoint file; give the guest that file and nothing else:

```sh
bridge-guest --provider hytale --id "Server/Models/Beast/Rat.json" --in-front --endpoint-file /path/the/host/wrote.json
```

The file must be a regular file that only its owner can read (a guest refuses a looser one on macOS and Linux), and it is read again before
every connection attempt. Without a token, or with a wrong one, the guest prints `handshake failed: peer said bye: unauthorized` and tries
again twice a second until `--seconds` ends or you stop it. The token is never printed. `--token-file` takes a file whose first line is the
token, with `--port`, for a host whose token you set yourself.

Once a second it prints cameras and frames per second and the latency from "camera received" to "frame sent", and a summary at the end.

Ids that work (verified on the author's installs): Hytale `Server/Models/Intelligent/Trork/Trork.json` (clips `Walk`, `Run`, `Idle`, ...),
War Thunder `tanks/usa_m4a1_76w_sherman.grp:m4a1_76w_sherman`, Warframe `/Lotus/Characters/Tenno/Excalibur/Excalibur_skel.fbx`
(clips come from `?animations=<filter>`; pass the same filters as the `bridge-testbed` config). Nocturne `dvl0x01` (a demon with 5 clips) was run with `--game` set to the folder that holds `smt3hd_Data`. Minecraft is opened the same way (`--provider mc --id block/stone`, `--id item/diamond_sword`; blocks and items are props, 1 block = 1 m, no animation); `block/oak_stairs` was drawn into `guest_compose` and stands on its ground with its pixel-art textures (linearly filtered).
Any id of a provider's `list` that `load_animated` accepts should work.

### Warframe ids

`--id` takes the id the provider lists: the path of a model (`/Lotus/Characters/Tenno/Excalibur/Excalibur_skel.fbx`),
optionally with `?animations=<filter>`. A **uniqueName** is not an id: `--id /Lotus/Powersuits/Excalibur/Excalibur` stops
with `Warframe could not load ...: ... is not in H.Misc`. wf-unpack's `resolve` command (a release after 0.4.0; the
`wf` pin below is 0.3.1, but the command is a separate program and any build of it works) turns a uniqueName into the
ids it reaches, or says why there are none.

Run on 2026-11-05 on macOS arm64 (Metal) with a Steam install of build 25757057, the `wf` provider as pinned, against
rewrite-bridge's `fake_host` (`--port 47871`), for 10 s:

```sh
bridge-guest --provider wf --game /path/to/Cache.Windows --port 47871 --in-front --max-texture 512 --size 320x240 --seconds 10 \
  --id "/Lotus/Characters/Tenno/Excalibur/Excalibur_skel.fbx?animations=/Lotus/Animations/Tenno/Movement/MeleeWeapon/Sword/Jog*" \
  --clip /Lotus/Animations/Tenno/Movement/MeleeWeapon/Sword/JogForward_anim.fbx
```

The guest's summary: 510 cameras, 497 frames in 10.3 s (48.2 fps), camera received to frame sent mean 5.2 ms, p95 8.3 ms;
peak resident memory 0.75 GB (`/usr/bin/time -l`). `fake_host` logged the spawn with the id above, `malformed 0 stale 0
foreign 0 unknown_entity 0`, and a frame it wrote with `--dump` shows a jogging Excalibur standing on the host's floor
beside the host's pillar (one frame was looked at; occlusion by the pillar was not checked). Not run: a Warframe host, a Warframe client of any kind, a level as a scene (this
guest has no scenes), another build of the game.

Providers, as pinned here:

| `--provider` | reader | notes |
|---|---|---|
| `wt` | [wt-unpack](https://github.com/patodois/wt-unpack) `0.3.0` (`c06c71c`) | vehicles |
| `wf` | [wf-unpack](https://github.com/patodois/wf-unpack) `0.3.1` (`c482a56`) | models and animated ids. A Warframe model carries **no sky by default** (the guest draws its own neutral stage anyway) |
| `hytale` | [hytale-unpack](https://github.com/patodois/hytale-unpack) `0.2.1` (`c29e8db`) | entity models with clips. An entity now **wears its default attachments** (hats, hair, eyes, clothes), merged into the body's nodes, so a character is no longer bare |
| `nocturne` | [nocturne-unpack](https://github.com/patodois/nocturne-unpack) `0.2.0` (`355d578`) | demons with the clips of their animator |
| `mc` | [mc-unpack](https://github.com/patodois/mc-unpack) `0.3.0` (`43ef7ef`) | Minecraft: Java Edition blocks and items from the client jar |

**Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.** The `mc` provider reads the models of a client jar that
you already own; this project contains no Mojang code and no Mojang assets.

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

A host that requires a token: rewrite-bridge's `interop_peer host --port 47890 --endpoint FILE --label x` makes the token and writes `FILE` (start it with
`umask 077`, in a directory only you can use). Its session is scripted, so it reports that this guest did not follow it, but the handshake shows: with `--endpoint-file FILE`
the guest prints `connected`, and without the token, or with a wrong one, `handshake failed: peer said bye: unauthorized`.

## Limits

- One surface per pixel (that is what a depth plane holds): translucent materials become cut-outs (alpha 0.5), glass is not glass.
- No anti-aliasing: alpha is 0 or 1, so edges are jagged at low resolution. Render larger (`--size`) for smoother edges.
- Perspective hosts only. No shadows onto the host's world, no host lighting, no reflections of it.
- A clip with root motion walks the model away from the pose the host knows, and loops back. Use in-place clips.
- One entity, no scenes, no sockets or attachments, no input handling (the link's input focus is ignored).
- `--window` is for looking: it runs at the display's rate and adds latency.
- The model's bounds for standing on the ground are those of the rest pose.

## Dependencies

`rewrite-bridge` 0.4.0 (with its `link` feature, which has the token) and `rewrite-bridge-bevy` 0.4.0 from crates.io, Bevy 0.20, and the five providers by git at pinned
revisions (table above). The providers take `rewrite-bridge` `0.4` from crates.io too, so `Cargo.lock` holds exactly one copy of it
(`grep -c 'name = "rewrite-bridge"$' Cargo.lock` prints 1). Before 1.0 a new minor number is a different crate, so a provider revision that still asks for 0.3 does not
build with this one. `wf-unpack` at these revisions no longer uses `oozextract`, so that crate is not in `Cargo.lock` (`cargo tree --locked -i oozextract` finds nothing).

License: MIT OR Apache-2.0.
