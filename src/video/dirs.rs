use super::cfg;

use anyhow::{anyhow, Context};
use const_format::formatcp;
use log::*;
use once_cell::sync::OnceCell;
use std::{
    cmp::Eq,
    collections::HashMap,
    fmt, fs,
    path::{PathBuf, MAIN_SEPARATOR},
};

pub const OTR_DEFAULT_DIR: &str = "OTR";

const SUB_PATH_ROOT: &str = "";
const SUB_PATH_ENCODED: &str = "Encoded";
const SUB_PATH_DECODED: &str = "Decoded";
const SUB_PATH_CUT: &str = "Cut";
const SUB_PATH_ARCHIVE: &str = formatcp!("{}{}Archive", SUB_PATH_DECODED, MAIN_SEPARATOR);

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
    fn relative_path<'a>(&self) -> &'a str {
        match self {
            DirKind::Root => SUB_PATH_ROOT,
            DirKind::Encoded => SUB_PATH_ENCODED,
            DirKind::Decoded => SUB_PATH_DECODED,
            DirKind::Cut => SUB_PATH_CUT,
            DirKind::Archive => SUB_PATH_ARCHIVE,
        }
    }
}

/// Working sub directories (i.e., the sub directories for encoded, decoded, cut
/// etc. videos). The directory paths are determined once only and stored in a
/// static variable. While doing so, the directories are created if they do not
/// exist
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
                fs::create_dir_all(&sub_dir).with_context(|| {
                    format!("Could not create sub directory \"{}\"", sub_dir.display())
                })?;
                kind_to_path.insert(dir_kind, sub_dir);
            }
            Ok(kind_to_path)
        })
    }

    let dirs = working_sub_dir_create()
        .with_context(|| format!("Could not determine sub directory of kind \"{}\"", kind))?;

    if let Some(path) = dirs.get(kind) {
        return Ok(path);
    }
    Err(anyhow!(
        "Sub directory of kind \"{}\" not found",
        kind.to_string()
    ))
}

/// (Root) working directory. It is set to the working dir path which was
/// retrieved from the configuration. If there is no dir configured, the default
/// working dir is used, which is <VIDEO_DIR_OF_YOUR_OS>/OTR. The determination
/// is only done once. The result is stored in a static variable
pub fn working_dir() -> anyhow::Result<&'static PathBuf> {
    static WORKING_DIR: OnceCell<PathBuf> = OnceCell::new();
    WORKING_DIR.get_or_try_init(|| {
        let dir = if let Some(_dir) = cfg::working_dir() {
            trace!("Working directory retrieved from configuration file");
            _dir.to_path_buf()
        } else {
            trace!("No working directory configured: Try default directory");

            if let Some(video_dir) = dirs::video_dir() {
                trace!("Video directory determined: Assemble default working directory");
                video_dir.join(OTR_DEFAULT_DIR)
            } else {
                trace!("Video directory could not be determined");
                return Err(anyhow!("Working directory could not be determined"));
            }
        };

        debug!("Working directory: {:?}", dir);

        Ok(dir)
    })
}
