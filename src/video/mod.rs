mod cfg;
mod collecting;
mod cutting;
mod decoding;
mod dirs;

pub use collecting::collect;
pub use cutting::CutlistAccessType;

use anyhow::anyhow;
use dirs::DirKind;
use lazy_static::lazy_static;
use log::*;
use regex::Regex;
use std::{
    cmp, fmt, fs,
    path::{Path, PathBuf},
};

/// Key of an OTR video. That's the left part of the file name ending with
/// "_TVOON_DE". I.e., key of
/// Blue_in_the_Face_-_Alles_blauer_Dunst_22.01.08_22-00_one_85_TVOON_DE.mpg.HD.avi
/// is
/// Blue_in_the_Face_-_Alles_blauer_Dunst_22.01.08_22-00_one_85_TVOON_DE
#[derive(Clone, Eq, Hash, PartialEq, PartialOrd)]
pub struct Key(String);
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
/// Support conversion from &str to Key
impl From<&str> for Key {
    fn from(s: &str) -> Self {
        Key(s.to_string())
    }
}
/// Support conversion from String to Key
impl From<String> for Key {
    fn from(s: String) -> Self {
        Key(s)
    }
}

/// Status of a video - i.e., whether its encoded, decoded or cut. The status
/// can be ordered: Encoded < Decoded < Cut
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub enum Status {
    Encoded,
    Decoded,
    Cut,
}
impl PartialOrd for Status {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        if self == other {
            return Some(cmp::Ordering::Equal);
        }
        if *self == Status::Cut || (*self == Status::Decoded && *other == Status::Encoded) {
            return Some(cmp::Ordering::Greater);
        }
        Some(cmp::Ordering::Less)
    }
}
/// Support iteration over status value: Encoded -> Decoded -> Cut -> None
impl Iterator for Status {
    type Item = Status;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Status::Encoded => Some(Status::Decoded),
            Status::Decoded => Some(Status::Cut),
            Status::Cut => None,
        }
    }
}
impl Status {
    /// Map a video status to the corresponding directory kind
    fn as_dir_kind(self) -> DirKind {
        match self {
            Status::Encoded => DirKind::Encoded,
            Status::Decoded => DirKind::Decoded,
            Status::Cut => DirKind::Cut,
        }
    }
}

/// Video file downloaded from OTR, incl. its path, key and status
pub struct Video {
    p: PathBuf,
    k: Key,
    s: Status,
    e: Option<anyhow::Error>,
}

// Regular expressions to analyze video file names
lazy_static! {
    // Analyze the name of a (potential) video file that is not cut -
    // i.e., either encoded or decoded.
    static ref RE_UNCUT_VIDEO: Regex =
        Regex::new(r"^([^\.]+_\d{2}.\d{2}.\d{2}_\d{2}-\d{2}_[^_]+_\d+_TVOON_DE)\.[^\.]+(?P<fmt>\.(HQ|HD))?(?P<ext>\.[^\.]+)(?P<encext>\.otrkey)?$").unwrap();
    // Analyze the name of a (potential) video file that is cut
    static ref RE_CUT_VIDEO: Regex =
        Regex::new(r"^([^\.]+_\d{2}.\d{2}.\d{2}_\d{2}-\d{2}_[^_]+_\d+_TVOON_DE)\.(.*cut\..+)$").unwrap();
}

/// Support conversion of a &Path into a Video. Usage of From trait is not
/// possible since not all paths represent OTR videos and thus, an error can
/// occur
impl TryFrom<&Path> for Video {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        if let Some(file_name) = path.file_name() {
            if let Some(file_name_str) = file_name.to_str() {
                // Check if path represents a cut video file (the check for cut
                // video files must be done before the check for uncut video
                // file since cut video files in some cases also match the
                // regex for uncut files)
                if RE_CUT_VIDEO.is_match(file_name_str) {
                    // Assemble Video instance
                    let captures = RE_CUT_VIDEO.captures(file_name_str).unwrap();
                    let appendix = captures
                        .get(2)
                        .unwrap()
                        .as_str()
                        .replace("cut.", "")
                        .replace(".mpg", "");
                    return Ok(Video {
                        p: path.to_path_buf(),
                        k: Key::from(
                            captures.get(1).unwrap().as_str().to_string()
                                + if appendix.starts_with('.') { "" } else { "." }
                                + &appendix,
                        ),
                        s: Status::Cut,
                        e: None,
                    });
                }
                // Check if path represents an encoded or decoded video file
                if RE_UNCUT_VIDEO.is_match(file_name_str) {
                    // Assemble Video instance
                    let captures = RE_UNCUT_VIDEO.captures(file_name_str).unwrap();
                    return Ok(Video {
                        p: path.to_path_buf(),
                        k: Key::from(
                            captures.get(1).unwrap().as_str().to_string()
                                + if let Some(fmt) = captures.name("fmt") {
                                    fmt.as_str()
                                } else {
                                    ""
                                }
                                + captures.name("ext").unwrap().as_str(),
                        ),
                        s: if captures.name("encext").is_some() {
                            Status::Encoded
                        } else {
                            Status::Decoded
                        },
                        e: None,
                    });
                }
            }
        }
        Err(anyhow!("{:?} is not a valid video file", path))
    }
}

/// Support ordering of videos: By key (ascending), status (descending)
impl Eq for Video {}
impl Ord for Video {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self.key() < other.key() {
            return cmp::Ordering::Less;
        };
        if self.key() > other.key() {
            return cmp::Ordering::Greater;
        };
        if self.status() > other.status() {
            return cmp::Ordering::Less;
        };
        if self.status() < other.status() {
            return cmp::Ordering::Greater;
        };
        cmp::Ordering::Equal
    }
}
impl PartialEq for Video {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key() && self.status() == other.status()
    }
}
impl PartialOrd for Video {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Support conversion of Video to Path ref
impl AsRef<Path> for Video {
    fn as_ref(&self) -> &Path {
        &self.p
    }
}

impl Video {
    // Key of a Video
    pub fn key(&self) -> &Key {
        &self.k
    }

    // Status of a Video
    pub fn status(&self) -> Status {
        self.s
    }

    // Video error
    pub fn error(&self) -> &Option<anyhow::Error> {
        &self.e
    }

    // File name of a Video (i.e., the last part of its path)
    pub fn file_name(&self) -> &str {
        self.p.file_name().unwrap().to_str().unwrap()
    }

    // True if video is already cut, false otherwise.
    pub fn is_processed(&self) -> bool {
        self.status() == Status::Cut
    }

    /// Cut a decoded Video. The video status and path is updated accordingly. The
    /// video file is moved accordingly.
    /// The real thing is done by _cut, the private counterpart function.
    pub fn cut(&mut self, cutlist_access: CutlistAccessType) {
        if let Err(err) = self._cut(cutlist_access) {
            self.e = Some(err)
        }
    }

    /// Decode an encoded video. The video status and path is updated
    /// accordingly. The video file is moved accordingly.
    /// The real thing is done by _decode, the private counterpart function.
    pub fn decode(&mut self, user: Option<&str>, password: Option<&str>) {
        if let Err(err) = self._decode(user, password) {
            self.e = Some(err)
        }
    }

    // Path of the video it would have if it had the next status - i.e., the
    // decoded status if it is encoded now or the cut status if it is decoded
    // now. If the video is already cut, its current path is returned.
    fn next_path(&self) -> anyhow::Result<PathBuf> {
        match self.s {
            Status::Encoded => Ok(self
                .p
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(dirs::working_sub_dir(&(Status::Decoded).as_dir_kind())?)
                .join(self.file_name())
                .with_extension("")),
            Status::Decoded => Ok(self
                .p
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(dirs::working_sub_dir(&(Status::Cut).as_dir_kind())?)
                .join(self.file_name())
                .with_extension(
                    "cut".to_string() + "." + self.p.extension().unwrap().to_str().unwrap(),
                )),
            _ => Ok(self.p.to_path_buf()),
        }
    }

    // Changes the videos to the next status (i.e., if its in status encoded,
    // it is set to decoded, and if it is in status decoded it will be set to
    // cut). The video path is changed accordingly.
    fn change_to_next_status(&mut self) -> anyhow::Result<()> {
        if let Some(next_status) = self.s.next() {
            // NOTE: The new status must not we set before next_path() is
            //       executed first since next_path() uses the status !!!
            self.p = self.next_path()?;
            self.s = next_status;
        }
        Ok(())
    }

    /// Cut a decoded Video. The video status and path is updated accordingly. The
    /// video file is moved accordingly. Private cut function which is
    /// wrapped by its public counterpart.
    fn _cut(&mut self, cutlist_access: CutlistAccessType) -> anyhow::Result<()> {
        // nothing to do if video is not in status "decoded"
        if self.status() != Status::Decoded {
            return Ok(());
        }

        info!("Cutting {:?} ...", self.file_name());

        // Execute cutting of video
        if let Err(err) = cutting::cut(&self, self.next_path()?, cutlist_access) {
            return match err {
                cutting::CutError::NoCutlist => {
                    Err(anyhow!("No cutlist exists for {:?}", self.file_name()))
                }
                cutting::CutError::Any(err) => {
                    Err(err.context(format!("Could not cut {:?}", self.file_name())))
                }
                cutting::CutError::Default => Err(anyhow!(
                    "Could not cut {:?} for an unknown reason",
                    self.file_name()
                )),
            };
        }

        // In case of having cut the video successfully, move decoded video to
        // archive directory. Otherwise return with error
        if let Err(err) = fs::rename(
            &self.p,
            dirs::working_sub_dir(&DirKind::Archive)
                .unwrap()
                .join(self.file_name()),
        ) {
            error!(
                "{:?}",
                anyhow!(err).context(format!(
                    "Could not move {:?} to archive directory after successful cutting",
                    self.file_name()
                ))
            );
        }

        // Update video (status, path)
        self.change_to_next_status()?;

        info!("Cut {:?}", self.file_name());

        Ok(())
    }

    /// Decode an encoded video. The video status and path is updated accordingly.
    /// The video file is moved accordingly. Private decode function which is
    /// wrapped by its public counterpart.
    fn _decode(&mut self, user: Option<&str>, password: Option<&str>) -> anyhow::Result<()> {
        // Nothing to do if video is not in status "encoded"
        if self.status() != Status::Encoded {
            return Ok(());
        }

        let (user, password) = cfg::otr_access_data(user, password)?;

        info!("Decoding {:?} ...", self.file_name());

        // Execute decoding
        decoding::decode(&self, &self.next_path()?, user, password)?;

        info!("Decoded {:?}", self.file_name());

        // Update video (status, path)
        self.change_to_next_status()?;

        Ok(())
    }

    /// Moves a video file to the working sub directory corresponding to the status
    /// of the video. The Video (i.e., its path) is changed accordingly.
    fn move_to_working_dir(&mut self) -> anyhow::Result<()> {
        // Since video path was already checked for compliance before, it is OK to
        // simply unwrap the result
        let source_dir = self.p.parent().unwrap();

        let target_dir = dirs::working_sub_dir(&(self.status()).as_dir_kind())?;
        let target_path = target_dir.join(self.file_name());

        // Nothing to do if video is already in correct directory
        if source_dir == target_dir {
            return Ok(());
        }

        // Copy video file to working sub directory and adjust path
        fs::rename(&self.p, &target_path)?;
        self.p = target_path;

        Ok(())
    }
}
