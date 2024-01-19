use anyhow::{anyhow, Context};
use once_cell::sync::OnceCell;
use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

/// Name of configuration file
const CFG_FILENAME: &str = "otr.json";

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

/// Return working directory from configuration file
pub fn working_dir() -> anyhow::Result<Option<&'static Path>> {
    Ok(cfg_from_file()?.working_dir.as_deref())
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
