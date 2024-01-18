use anyhow::{anyhow, Context};
use log::*;
use once_cell::sync::OnceCell;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use super::dirs::DEFAULT_WORKING_DIR;

const CFG_FILENAME: &str = "otr.json";

/// Set the default configuration directory depending on the OS. Currently only
/// macOS and Linux are supported. Thus, if compilation is done on a different
/// OS, an error is thrown
#[cfg(target_os = "linux")]
const CFG_DEFAULT_DIR: &str = ".config";
#[cfg(target_os = "macos")]
const CFG_DEFAULT_DIR: &str = "Library/Application Support";

/// Determine OTR access data (user, password). First, it is tried to retrieve
/// that data from the command line arguments. If these do not contain the access
/// data, they are tried to retrieved from the configuration file. The result is
/// stored in a static variable. Thus, data is only determined once.
pub fn otr_access_data<'a>(
    user: Option<&'a str>,
    password: Option<&'a str>,
) -> anyhow::Result<(&'a str, &'a str)> {
    Ok((
        if let Some(_user) = user {
            _user
        } else {
            let cfg = cfg_from_file()?;
            if let Some(user) = &cfg.user {
                user.as_str()
            } else {
                return Err(anyhow!("OTR user name is not configured"));
            }
        },
        if let Some(_password) = password {
            _password
        } else {
            let cfg = cfg_from_file()?;
            if let Some(password) = &cfg.password {
                password.as_str()
            } else {
                return Err(anyhow!("OTR password is not configured"));
            }
        },
    ))
}

/// (Root) working directory. It is set to dir provided dir is set. Otherwise, it
/// is set to the working dir path which was retrieved from the configuration
/// file. If there is no dir configured, the default workking dir is used, which
/// is <VIDEO_DIR_OF_YOUR_OS>/OTR. The determination is only done once. The
/// result is stored in a static variable.
pub fn working_dir(dir: Option<&Path>) -> anyhow::Result<&'static PathBuf> {
    static WORKING_DIR: OnceCell<PathBuf> = OnceCell::new();
    WORKING_DIR.get_or_try_init(|| {
        if let Some(_dir) = dir {
            return Ok(_dir.to_path_buf());
        }

        trace!("No working directory submitted: Try configuration file");

        if let Some(_dir) = &cfg_from_file()?.working_dir {
            trace!("Working directory retrieved from configuration file");
            return Ok(_dir.to_path_buf());
        }

        trace!("No working directory configured: Try default directory");

        if let Some(video_dir) = dirs::video_dir() {
            trace!("Video directory determined: Assemble default working directory");
            return Ok(video_dir.join(DEFAULT_WORKING_DIR));
        }

        debug!("Video directory could not be determined");

        Err(anyhow!("Working directory could not be determined"))
    })
}

/// Content of the configuration file
#[derive(serde::Deserialize, Debug, Default)]
struct CfgFromFile {
    working_dir: Option<PathBuf>,
    user: Option<String>,
    password: Option<String>,
}

/// Retrieve the content of the configuration file. That is only done once. The
/// result is stored in a static variable.
fn cfg_from_file() -> anyhow::Result<&'static CfgFromFile> {
    static CFG_FROM_FILE: OnceCell<CfgFromFile> = OnceCell::new();
    CFG_FROM_FILE.get_or_try_init(|| {
        // Assemble path for config file. Sequence:
        //   (1) Standard configuration directory of the OS (if that's available)
        //   (2) Home directory of the OS (if that's available) joined with
        //       the default (relative) configuration path (constants defined at
        //       beginning of this file)
        let path = if let Some(cfg_dir) = dirs::config_dir() {
            cfg_dir.join(CFG_FILENAME)
        } else if let Some(home_dir) = dirs::home_dir() {
            home_dir.join(CFG_DEFAULT_DIR).join(CFG_FILENAME)
        } else {
            return Err(anyhow!("could not determine path of configuration file"));
        };

        // Parse config file
        let file = File::open(&path)
            .with_context(|| format!("could not open configuration file {:?}", path))?;
        let cfg = serde_json::from_reader(BufReader::new(file))
            .with_context(|| format!("could not read configuration file {:?}", path))?;
        Ok(cfg)
    })
}
