use anyhow::{anyhow, Context};
use once_cell::sync::OnceCell;
use std::{cmp::Eq, collections::HashMap, fmt, fs, fs::File, io::BufReader, path::PathBuf};
use structopt::StructOpt;

const CFG_FILENAME: &str = "otr.json";
const CFG_DEFAULT_DIR: &str = ".config";

const SUB_PATH_ROOT: &str = "";
const SUB_PATH_ENCODED: &str = "Encoded";
const SUB_PATH_DECODED: &str = "Decoded";
const SUB_PATH_CUT: &str = "Cut";
const SUB_PATH_ARCHIVE: &str = "Decoded/Archive";

/// Args holds the command line arguments
#[derive(StructOpt, Debug)]
#[structopt(
    name = "otr",
    version = "0.2.4",
    author = "Michael Picht <mipi@fsfe.org>",
    about = "otr decodes and cuts video files that were downloaded from Online TV Recorder <https://onlinetvrecorder.com/>"
)]
struct Args {
    #[structopt(
        short = "c",
        long = "config",
        help = "Path of config file (default is ~/.config/otr.json)",
        parse(from_os_str)
    )]
    cfg_file_path: Option<PathBuf>,
    #[structopt(
        short = "d",
        long = "directory",
        help = "Working directory (overwrites config file content)",
        parse(from_os_str)
    )]
    working_dir: Option<PathBuf>,
    #[structopt(
        short = "u",
        long = "user",
        help = "User name for Online TV Recorder (overwrites config file content)"
    )]
    user: Option<String>,
    #[structopt(
        short = "p",
        long = "password",
        help = "Password for Online TV Recorder (overwrites config file content)"
    )]
    password: Option<String>,
    #[structopt(parse(from_os_str))]
    videos: Vec<std::path::PathBuf>,
}

/// DirKind represents different directory types
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

/// dir_kind_to_sub_path returns the relative sub directory path for each
/// directory kind
pub fn dir_kind_to_sub_path<'a>(dir_kind: &DirKind) -> &'a str {
    match dir_kind {
        DirKind::Root => SUB_PATH_ROOT,
        DirKind::Encoded => SUB_PATH_ENCODED,
        DirKind::Decoded => SUB_PATH_DECODED,
        DirKind::Cut => SUB_PATH_CUT,
        DirKind::Archive => SUB_PATH_ARCHIVE,
    }
}

/// user and password for accessing service provided by the OTR website
#[derive(Debug, Default)]
pub struct OTRAccessData {
    pub user: String,
    pub password: String,
}

/// determination of OTR access data. First, it is tried to retrieve that
/// data from the command line arguments. If these do not contain the access
/// data, they are tried to retrieved from the configuration file.
/// Result is stored in a static variable. Thus, data is only determined once.
pub fn otr_access_data() -> anyhow::Result<OTRAccessData> {
    // retrieve OTR user and password
    let data = OTRAccessData {
        user: if let Some(user) = &args().user {
            user.clone()
        } else {
            let cfg = cfg_from_file()?;
            if let Some(user) = &cfg.user {
                user.clone()
            } else {
                return Err(anyhow!("OTR user name is not configured"));
            }
        },
        password: if let Some(password) = &args().password {
            password.clone()
        } else {
            let cfg = cfg_from_file()?;
            if let Some(password) = &cfg.password {
                password.clone()
            } else {
                return Err(anyhow!("OTR password is not configured"));
            }
        },
    };

    Ok(data)
}

/// returns a vector of videos whose paths have been submitted as command line
/// arguments
pub fn videos() -> &'static Vec<PathBuf> {
    &args().videos
}

/// determins working sub directories (i.e., the sub directories for encoded,
/// decoded, cut etc. videos). The directory paths are determined once only and
/// stored in a static variable
pub fn working_sub_dir(kind: &DirKind) -> anyhow::Result<&'static PathBuf> {
    fn working_sub_dir_create() -> anyhow::Result<&'static HashMap<DirKind, PathBuf>> {
        static WORKING_SUB_DIRS: OnceCell<HashMap<DirKind, PathBuf>> = OnceCell::new();
        WORKING_SUB_DIRS.get_or_try_init(|| {
            let mut kind_to_path: HashMap<DirKind, PathBuf> = HashMap::new();
            let working_dir = working_dir()?;
            for dir_kind in vec![
                DirKind::Root,
                DirKind::Encoded,
                DirKind::Decoded,
                DirKind::Cut,
                DirKind::Archive,
            ] {
                let sub_dir = working_dir.join(dir_kind_to_sub_path(&dir_kind));
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

/// returns command line arguments via Args structure. The conversion /
/// determination into that structure is done once only. The result is
/// stored in a static variable
fn args() -> &'static Args {
    static ARGS: OnceCell<Args> = OnceCell::new();
    ARGS.get_or_init(Args::from_args)
}

/// structure to hold content of configuration file
#[derive(serde::Deserialize, Debug, Default)]
struct CfgFromFile {
    working_dir: Option<PathBuf>,
    user: Option<String>,
    password: Option<String>,
}

/// retrieves content of configuration file. That is only done once. The result
/// is stored in a static variable
fn cfg_from_file() -> anyhow::Result<&'static CfgFromFile> {
    static CFG_FROM_FILE: OnceCell<CfgFromFile> = OnceCell::new();
    CFG_FROM_FILE.get_or_try_init(|| {
        // assemble path for config file. Sequence:
        //   (1) command line arguments
        //   (2) XDG config dir (if that's available)
        //   (3) XDG home dir (if that's available) joined with default
        //       (relative) configuration path
        let path = if let Some(path) = &args().cfg_file_path {
            path.to_path_buf()
        } else if let Some(cfg_dir) = dirs::config_dir() {
            cfg_dir.join(CFG_FILENAME)
        } else if let Some(home_dir) = dirs::home_dir() {
            home_dir.join(CFG_DEFAULT_DIR).join(CFG_FILENAME)
        } else {
            return Err(anyhow!("could not determine path of configuration file"));
        };

        // parse config file
        let file = File::open(&path)
            .with_context(|| format!("could not open configuration file {:?}", path))?;
        let cfg = serde_json::from_reader(BufReader::new(file))
            .with_context(|| format!("could not read configuration file {:?}", path))?;
        Ok(cfg)
    })
}

/// determination of (root) working directory. First, it is tried to get it from
/// the command line arguments. If that is not successful, it is tried
/// to get from the configuration file.
/// The determination is only done once. The result is stored in a static variable
fn working_dir() -> anyhow::Result<&'static PathBuf> {
    static WORKING_DIR: OnceCell<PathBuf> = OnceCell::new();
    WORKING_DIR.get_or_try_init(|| {
        if let Some(dir) = &args().working_dir {
            return Ok(dir.to_path_buf());
        }
        let cfg = cfg_from_file()?;
        if let Some(dir) = &cfg.working_dir {
            return Ok(dir.to_path_buf());
        }
        if let Some(dir) = &cfg.working_dir {
            return Ok(dir.to_path_buf());
        }
        Err(anyhow!("working directory is not configured"))
    })
}
