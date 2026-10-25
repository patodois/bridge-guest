use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ProviderName {
    /// War Thunder (the game folder)
    Wt,
    /// Warframe (the Cache.Windows folder)
    Wf,
    /// Hytale (Assets.zip; without --game, the launcher's install)
    Hytale,
    /// Shin Megami Tensei III Nocturne HD (the folder that holds smt3hd_Data)
    Nocturne,
    /// Minecraft: Java Edition (a client jar; without --game, the newest one a launcher installed)
    Mc,
    /// Two flat planes with exact geometry, for measuring depth (no game needed)
    #[value(hide = true)]
    Selftest,
}

#[derive(Parser, Debug, Clone)]
#[command(
    version,
    about = "Puts a rewrite-bridge provider's model into a game that hosts the passthrough link",
    long_about = "Connects to a host game as the guest of the passthrough link (rewrite-bridge, \
docs/CONTRACT.md), spawns one model from a provider, and for every camera the host sends renders \
the model on the GPU with exactly that camera and answers with colour and linear depth, so the host \
can put it in its world and hide it behind its own geometry."
)]
pub struct Opts {
    /// Which game's assets to draw from
    #[arg(long, value_enum)]
    pub provider: ProviderName,

    /// The game's files (see --provider). Hytale and Minecraft may leave it out
    #[arg(long)]
    pub game: Option<PathBuf>,

    /// Asset id, as the provider lists it (see --list)
    #[arg(long, required_unless_present = "list")]
    pub id: Option<String>,

    /// Print the ids that contain this text (all with "") and exit
    #[arg(long, value_name = "FILTER")]
    pub list: Option<String>,

    /// Print the clip names of --id and exit
    #[arg(long)]
    pub list_clips: bool,

    /// Animation clip to loop (the model stands in its rest pose without one)
    #[arg(long)]
    pub clip: Option<String>,

    /// The host's control port
    #[arg(long, default_value_t = 47800)]
    pub port: u16,

    /// Largest image to render and send, WxH; the host's aspect ratio is kept inside it
    #[arg(long, default_value = "640x360", value_parser = parse_size)]
    pub size: (u32, u32),

    /// Stand ahead of the first camera, on the host's ground, facing it, instead of at the origin
    #[arg(long)]
    pub in_front: bool,

    /// Metres from the camera with --in-front (default: 4, or more for a big model)
    #[arg(long)]
    pub distance: Option<f32>,

    /// Which way the model faces, degrees about +Y (0 = its front toward +Z). With --in-front the
    /// default is to face the camera
    #[arg(long)]
    pub yaw: Option<f32>,

    /// Size of the model relative to the provider's metres
    #[arg(long, default_value_t = 1.0)]
    pub scale: f32,

    /// Warframe: largest side of a baked texture, pixels (0 = full size)
    #[arg(long, default_value_t = 1024)]
    pub max_texture: u32,

    /// Also show what is rendered in a window
    #[arg(long)]
    pub window: bool,

    /// Quit after this many seconds
    #[arg(long)]
    pub seconds: Option<f32>,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or("expected WIDTHxHEIGHT, like 640x360")?;
    let w: u32 = w.trim().parse().map_err(|_| "bad width")?;
    let h: u32 = h.trim().parse().map_err(|_| "bad height")?;
    if !(16..=8192).contains(&w) || !(16..=8192).contains(&h) {
        return Err("each side must be 16 to 8192".into());
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse() {
        assert_eq!(parse_size("640x360"), Ok((640, 360)));
        assert!(parse_size("640").is_err());
        assert!(parse_size("8x8").is_err());
    }

    #[test]
    fn the_documented_command_line_parses() {
        let o = Opts::try_parse_from([
            "bridge-guest",
            "--provider",
            "wf",
            "--game",
            "/g",
            "--id",
            "x",
            "--clip",
            "walk",
            "--port",
            "47801",
            "--size",
            "800x450",
            "--in-front",
            "--scale",
            "2",
        ])
        .unwrap();
        assert_eq!(o.provider, ProviderName::Wf);
        assert_eq!(
            (o.port, o.size, o.in_front, o.scale),
            (47801, (800, 450), true, 2.0)
        );
        assert!(Opts::try_parse_from(["bridge-guest", "--provider", "wt"]).is_err());
        assert!(Opts::try_parse_from(["bridge-guest", "--provider", "wt", "--list", ""]).is_ok());
    }
}
