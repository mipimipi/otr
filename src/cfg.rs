use otr_utils::cutting::CutlistRating;

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

/// Returns the access token for cutlist.at. In case an error occurred while
/// reading the configuration data from the file, None is returned
pub fn cutlist_at_access_token() -> Option<&'static str> {
    match cfg_from_file() {
        Ok(cfg) => {
            if let Some(_cutting) = &cfg.cutting {
                _cutting.cutlist_at_access_token.as_deref()
            } else {
                None
            }
        }
        Err(err) => {
            warn!(
                "Cannot determine access token for cutlist.at from configuration: {:?}",
                err
            );
            None
        }
    }
}

/// Returns the default cut list rating from the configuration file. In case an
/// error occurred while reading the configuration data from the file, or no
/// rating is set, 0 is returned
pub fn cutlist_rating() -> CutlistRating {
    match cfg_from_file() {
        Ok(cfg) => {
            if let Some(_cutting) = &cfg.cutting {
                if let Some(_rating) = _cutting.cutlist_rating {
                    trace!("Cut list rating {} configured", _rating);
                    _rating
                } else {
                    trace!("No cut list rating configured");
                    0
                }
            } else {
                trace!("No cutting section configured");
                0
            }
        }
        Err(err) => {
            warn!(
                "Cannot determine cut list rating from configuration: {:?}",
                err
            );
            0
        }
    }
}

/// Returns the minimum cut list rating from the configuration file. In case an
/// error occurred while reading the configuration data from the file, None is
/// returned
pub fn min_cutlist_rating() -> Option<CutlistRating> {
    match cfg_from_file() {
        Ok(cfg) => {
            if let Some(_cutting) = &cfg.cutting {
                _cutting.min_cutlist_rating
            } else {
                trace!("No cutting section configured");
                None
            }
        }
        Err(err) => {
            warn!(
                "Cannot determine minimum cut list rating from configuration: {:?}",
                err
            );
            None
        }
    }
}

/// Returns OTR access data (i.e., user and password) that were maintained in
/// the configuration file.  In case an error occurred while reading the
/// configuration data from the file, None is returned. Warnings are logged if
/// either user or password is not maintained. This is done because this function
/// is only called if this data is required
pub fn otr_access_data() -> Option<(&'static str, &'static str)> {
    match cfg_from_file() {
        Ok(cfg) => match &cfg.decoding {
            None => {
                warn!("OTR access data is not maintained in configuration file");
                None
            }
            Some(_decoding) => {
                if _decoding.user.is_none() {
                    warn!("OTR user is not maintained in configuration file");
                    None
                } else if _decoding.password.is_none() {
                    warn!("OTR password is not maintained in configuration file");
                    None
                } else {
                    Some((
                        _decoding.user.as_ref().unwrap(),
                        _decoding.password.as_ref().unwrap(),
                    ))
                }
            }
        },
        Err(err) => {
            warn!(
                "Cannot determine OTR access data from configuration file: {:?}",
                err
            );
            None
        }
    }
}

/// Returns a flag that determines whether cut lists shall be suibmitted to
/// cutlist.at from the configuration file. In case an  error occurred while
/// reading the configuration data from the file, or if the flag is not
/// maintained, false is returned
pub fn submit_cutlists() -> bool {
    match cfg_from_file() {
        Ok(cfg) => {
            if let Some(_cutting) = &cfg.cutting {
                _cutting.submit_cutlists.unwrap_or_default()
            } else {
                false
            }
        }
        Err(err) => {
            trace!(
                "Set submit_cutlists to false since it cannot be determined from configuration: {:?}",
                err
            );
            false
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
    decoding: Option<Decoding>,
    cutting: Option<Cutting>,
}
#[derive(serde::Deserialize, Debug, Default)]
struct Decoding {
    user: Option<String>,
    password: Option<String>,
}
#[derive(serde::Deserialize, Debug, Default)]
struct Cutting {
    min_cutlist_rating: Option<u8>,
    cutlist_rating: Option<u8>,
    submit_cutlists: Option<bool>,
    cutlist_at_access_token: Option<String>,
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
            .with_context(|| format!("could not open configuration file \"{}\"", path.display()))?;
        let cfg = serde_json::from_reader(BufReader::new(file))
            .with_context(|| format!("could not read configuration file \"{}\"", path.display()))?;

        Ok(cfg)
    })
}
