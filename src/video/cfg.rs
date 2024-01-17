use anyhow::{anyhow, Context};
use once_cell::sync::OnceCell;
use std::{cmp::Eq, collections::HashMap, fmt, fs, fs::File, io::BufReader, path::PathBuf};

use crate::cli;

const CFG_FILENAME: &str = "otr.json";

/// Set the default configuration directory depending on the OS. Currently only
/// macOS and Linux are supported. Thus, if compilation is done on a different
/// OS, an error is thrown
#[cfg(target_os = "linux")]
const CFG_DEFAULT_DIR: &str = ".config";
#[cfg(target_os = "macos")]
const CFG_DEFAULT_DIR: &str = "Library/Application Support";

const SUB_PATH_ROOT: &str = "";
const SUB_PATH_ENCODED: &str = "Encoded";
const SUB_PATH_DECODED: &str = "Decoded";
const SUB_PATH_CUT: &str = "Cut";
const SUB_PATH_ARCHIVE: &str = "Decoded/Archive";

/// Directory types
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum DirKind {
    Root,
    Encoded,
    Decoded,
    Cut,
    Archive,
}
impl fmt::Display for DirKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DirKind::Root => write!(f, "Root"),
            DirKind::Encoded => write!(f, "Encoded"),
            DirKind::Decoded => write!(f, "Decoded"),
            DirKind::Cut => write!(f, "Cut"),
            DirKind::Archive => write!(f, "Archive"),
        }
    }
}
impl DirKind {
    /// Relative path for each directory kind
    pub fn relative_path<'a>(&self) -> &'a str {
        match self {
            DirKind::Root => SUB_PATH_ROOT,
            DirKind::Encoded => SUB_PATH_ENCODED,
            DirKind::Decoded => SUB_PATH_DECODED,
            DirKind::Cut => SUB_PATH_CUT,
            DirKind::Archive => SUB_PATH_ARCHIVE,
        }
    }
}

/// Determine OTR access data (user, password). First, it is tried to retrieve that data from the
/// command line arguments. If these do not contain the access data, they are
/// tried to retrieved from the configuration file. The result is stored in a
/// static variable. Thus, data is only determined once.
pub fn otr_access_data<'clicfg>(
    user: Option<&'clicfg str>,
    password: Option<&'clicfg str>,
) -> anyhow::Result<(&'clicfg str, &'clicfg str)> {
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

/// Working sub directories (i.e., the sub directories for encoded, decoded, cut
/// etc. videos). The directory paths are determined once only and stored in a
/// static variable.
pub fn working_sub_dir(kind: &DirKind) -> anyhow::Result<&'static PathBuf> {
    fn working_sub_dir_create() -> anyhow::Result<&'static HashMap<DirKind, PathBuf>> {
        static WORKING_SUB_DIRS: OnceCell<HashMap<DirKind, PathBuf>> = OnceCell::new();
        WORKING_SUB_DIRS.get_or_try_init(|| {
            let mut kind_to_path: HashMap<DirKind, PathBuf> = HashMap::new();
            let working_dir = working_dir()?;
            for dir_kind in [
                DirKind::Root,
                DirKind::Encoded,
                DirKind::Decoded,
                DirKind::Cut,
                DirKind::Archive,
            ] {
                let sub_dir = working_dir.join(dir_kind.relative_path());
                fs::create_dir_all(&sub_dir)
                    .with_context(|| format!("could not create sub directory {:?}", sub_dir))?;
                kind_to_path.insert(dir_kind, sub_dir);
            }
            Ok(kind_to_path)
        })
    }

    let dirs = working_sub_dir_create().with_context(|| {
        format!(
            "could not determine sub directory of kind {:?}",
            kind.to_string()
        )
    })?;

    if let Some(path) = dirs.get(kind) {
        return Ok(path);
    }
    Err(anyhow!(
        "sub directory of kind {:?} not found",
        kind.to_string()
    ))
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
        //   (1) XDG config dir (if that's available)
        //   (2) XDG home dir (if that's available) joined with default
        //       (relative) configuration path
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

/// (Root) working directory. First, it is tried to get it from the command line
/// arguments. If that is not successful, it is tried to get from the
/// configuration file. The determination is only done once. The result is
/// stored in a static variable.
fn working_dir() -> anyhow::Result<&'static PathBuf> {
    static WORKING_DIR: OnceCell<PathBuf> = OnceCell::new();
    WORKING_DIR.get_or_try_init(|| {
        if let Some(dir) = &cli::args().working_dir {
            return Ok(dir.to_path_buf());
        }
        if let Some(dir) = &cfg_from_file()?.working_dir {
            return Ok(dir.to_path_buf());
        }
        Err(anyhow!("working directory is not configured"))
    })
}
