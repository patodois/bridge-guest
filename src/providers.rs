//! Opening the game readers.

use anyhow::{bail, Result};
use rewrite_bridge::Provider;

use crate::cli::{Opts, ProviderName};

pub type DynProvider = Box<dyn Provider + Send + Sync>;

pub fn open(opts: &Opts) -> Result<DynProvider> {
    // Each game's own error says what is wrong with its files; add which option named them
    let need = |what: &str| -> Result<std::path::PathBuf> {
        let p = opts
            .game
            .clone()
            .ok_or_else(|| anyhow::anyhow!("--game must be {what}"))?;
        anyhow::ensure!(p.exists(), "{} does not exist", p.display());
        Ok(p)
    };
    match opts.provider {
        ProviderName::Selftest => Ok(Box::new(crate::selftest::Selftest)),
        #[cfg(feature = "wt")]
        ProviderName::Wt => {
            let p = need("War Thunder's game folder")?;
            anyhow::ensure!(p.is_dir(), "{} is not a folder", p.display());
            Ok(Box::new(wt_unpack::bridge::WarThunder::new(p)))
        }
        #[cfg(feature = "wf")]
        ProviderName::Wf => {
            let p = need("Warframe's Cache.Windows folder")?;
            anyhow::ensure!(p.is_dir(), "{} is not a folder", p.display());
            let max = (opts.max_texture > 0).then_some(opts.max_texture);
            Ok(Box::new(
                wf_unpack::bridge::Warframe::open(&p)?.with_max_texture(max),
            ))
        }
        #[cfg(feature = "hytale")]
        ProviderName::Hytale => Ok(Box::new(match &opts.game {
            Some(p) => hytale_unpack::bridge::Hytale::open(p)?,
            None => hytale_unpack::bridge::Hytale::open_default()?,
        })),
        #[cfg(feature = "nocturne")]
        ProviderName::Nocturne => {
            let p = need("the folder that holds smt3hd_Data")?;
            anyhow::ensure!(p.is_dir(), "{} is not a folder", p.display());
            Ok(Box::new(nocturne_unpack::bridge::Nocturne::open(&p)?))
        }
        #[cfg(feature = "mc")]
        ProviderName::Mc => Ok(Box::new(match &opts.game {
            Some(p) => {
                anyhow::ensure!(p.exists(), "{} does not exist", p.display());
                mc_unpack::bridge::Minecraft::open(p)?
            }
            None => mc_unpack::bridge::Minecraft::open_default()?,
        })),
        #[allow(unreachable_patterns)]
        other => bail!("this build has no `{other:?}` provider (cargo feature of the same name)"),
    }
}
