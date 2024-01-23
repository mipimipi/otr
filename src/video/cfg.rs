use super::CutlistRating;

use anyhow::{anyhow, Context};
use log::*;
use once_cell::sync::OnceCell;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

/// Name of configuration file
const CFG_FILENAME: &str = "otr.json";

/// Returns the minimum cut list rating from the configuration file. In case an
/// error occurred while reading the configuration data from the file, None is
/// returned
pub fn min_cutlist_rating() -> Option<CutlistRating> {
    match cfg_from_file() {
        Ok(cfg) => cfg.min_cutlist_rating,
        Err(err) => {
            warn!(
                "Cannot determine minimum cut list rating from configuration: {:?}",
                err
            );
            None
        }
    }
}

/// Returns OTR access data (i.e., user ans password) that were maintained in
/// the configuration file.  In case an error occurred while reading the
/// configuration data from the file, None is returned. Warnings are logged if
/// either user or password is not maintained. This is done because this function
/// is only called if this data is required
pub fn otr_access_data() -> Option<(&'static str, &'static str)> {
    match cfg_from_file() {
        Ok(cfg) => {
            if cfg.user.is_none() {
                warn!("OTR user is not maintained in configuration file");
                None
            } else if cfg.password.is_none() {
                warn!("OTR password is not maintained in configuration file");
                None
            } else {
                Some((cfg.user.as_ref().unwrap(), cfg.password.as_ref().unwrap()))
            }
        }
        Err(err) => {
            warn!(
                "Cannot determine OTR access data from configuration file: {:?}",
                err
            );
            None
        }
    }
}

/// Returns the working directory from configuration file. In case an error
/// occurred while reading the configuration data from the file, None is
/// returned
pub fn working_dir() -> Option<&'static Path> {
    match cfg_from_file() {
        Ok(cfg) => cfg.working_dir.as_deref(),
        Err(err) => {
            warn!(
                "Cannot determine working directory from configuration file: {:?}",
                err
            );
            None
        }
    }
}

/// Content of the configuration file
#[derive(serde::Deserialize, Debug, Default)]
struct CfgFromFile {
    working_dir: Option<PathBuf>,
    user: Option<String>,
    password: Option<String>,
    min_cutlist_rating: Option<u8>,
}

/// Retrieve the content of the configuration file. That is only done once. The
/// result is stored in a static variable.
fn cfg_from_file() -> anyhow::Result<&'static CfgFromFile> {
    static CFG_FROM_FILE: OnceCell<CfgFromFile> = OnceCell::new();
    CFG_FROM_FILE.get_or_try_init(|| {
        // Assemble path for config file: Get standard configuration directory of
        // the OS (if that's available) and append the otr config file name
        let path = if let Some(cfg_dir) = dirs::config_dir() {
            cfg_dir.join(CFG_FILENAME)
        } else {
            return Err(anyhow!(
                "Could not determine path of configuration directory for this OS"
            ));
        };

        // Parse config file
        let file = File::open(&path)
            .with_context(|| format!("could not open configuration file {:?}", path))?;
        let cfg = serde_json::from_reader(BufReader::new(file))
            .with_context(|| format!("could not read configuration file {:?}", path))?;
        Ok(cfg)
    })
}
