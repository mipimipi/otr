mod cfg;
mod collecting;
mod cutting;
mod decoding;
mod dirs;

pub use collecting::collect;
pub use cutting::{mkvmerge_is_installed, CutlistAccessType, CutlistID, CutlistRating};
use cutting::{CutError, CutlistCtrl};

use anyhow::{anyhow, Context};
use dirs::DirKind;
use lazy_static::lazy_static;
use log::*;
use regex::Regex;
use std::{
    cmp, fmt, fs,
    marker::Copy,
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

    /// Cut a decoded Video. The video status and path is updated accordingly.
    /// The video file is moved accordingly. The real thing is done by _cut, the
    /// private counterpart function.
    /// cutlist_access specified how to (try to) get an appropriate cut list,
    /// min_cutlist_rating specifies the minimum rating a cutlist must have when
    /// automatically selected from the cut list provider
    pub fn cut(
        &mut self,
        cutlist_access: CutlistAccessType,
        cutlist_rating: Option<CutlistRating>,
        min_cutlist_rating: Option<CutlistRating>,
    ) {
        if let Err(err) = self._cut(cutlist_access, cutlist_rating, min_cutlist_rating) {
            self.e = Some(err)
        }
    }

    /// Decode an encoded video. The video status and path is updated
    /// accordingly. The video file is moved accordingly.
    /// The real thing is done by _decode, the private counterpart function.
    pub fn decode(&mut self, access_data: Option<(&'static str, &'static str)>) {
        if let Err(err) = self._decode(access_data) {
            self.e = Some(err)
        }
    }

    /// Create a new video from a video file path
    fn new<P>(path: P) -> anyhow::Result<Self>
    where
        P: Into<PathBuf> + Copy,
    {
        if let Some(file_name) = path.into().file_name() {
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
                    // Assemble Video instance
                    return Ok(Video {
                        p: fs::canonicalize(path.into()).context(format!(
                            "Could not create video from path {}",
                            path.into().display()
                        ))?,
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
                        p: fs::canonicalize(path.into()).context(format!(
                            "Could not create video from path {}",
                            path.into().display()
                        ))?,
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
        Err(anyhow!(
            "{} is not a valid video file",
            path.into().display()
        ))
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

    /// Cut a decoded Video (private cut function which is wrapped by its public
    /// counterpart). The video status and path, and the video file is moved
    /// accordingly.
    /// cutlist_access specifies how to (try to) get an appropriate cut list,
    /// min_cutlist_rating specifies the minimum rating a cutlist must have when
    /// automatically selected from the cut list provider
    fn _cut(
        &mut self,
        cutlist_access: CutlistAccessType,
        cutlist_rating: Option<CutlistRating>,
        min_cutlist_rating: Option<CutlistRating>,
    ) -> anyhow::Result<()> {
        // Nothing to do if video is not in status "decoded"
        if self.status() != Status::Decoded {
            return Ok(());
        }

        info!("Cutting \"{}\" ...", self.file_name());

        // Cut video and move cut video to corresponding directory
        match cutting::cut(
            &self,
            self.next_path()?,
            dirs::tmp_dir()?,
            &CutlistCtrl {
                access_type: cutlist_access,
                min_rating: min_cutlist_rating.or_else(cfg::min_cutlist_rating),
                rating: cutlist_rating.unwrap_or(cfg::cutlist_rating()),
                submit: cfg::submit_cutlists(),
                access_token: cfg::cutlist_at_access_token(),
            },
        ) {
            Ok(()) => {
                // In case the video was cut suceesfully and a (potential)
                // submission of the cut list was done successfully, move decoded
                // video to archive directory and return with Ok
                self.move_to_archive_dir()?;

                // Update video (status, path)
                self.change_to_next_status()?;

                info!("Cut \"{}\"", self.file_name());

                Ok(())
            }
            Err(CutError::CutlistSubmissionFailed(err)) => {
                // In case the video was cut successfully, but submission of cut
                // list failed, move decoded video to archive directory and
                // return with Error
                self.move_to_archive_dir()?;

                // Update video (status, path)
                self.change_to_next_status()?;

                info!("Cut \"{}\"", self.file_name());

                Err(err.context("Video was cut, but cut list could not be submitted to cutlist.at"))
            }
            Err(CutError::Any(err)) => Err(err.context("Could not cut video")),
            Err(CutError::Default) => Err(anyhow!("Could not cut video for an unknown reason")),
            Err(CutError::NoCutlist) => Err(anyhow!("No cut list exists for video")),
        }
    }

    /// Decode an encoded video (private decode function which is wrapped by its
    /// public counterpart). The video status and path is updated accordingly,
    /// and the video file is moved accordingly.
    fn _decode(&mut self, access_data: Option<(&'static str, &'static str)>) -> anyhow::Result<()> {
        // Nothing to do if video is not in status "encoded"
        if self.status() != Status::Encoded {
            return Ok(());
        }

        let (user, password) =
            if let Some((_user, _password)) = access_data.or_else(cfg::otr_access_data) {
                (_user, _password)
            } else {
                return Err(anyhow!("OTR user and password required to decode video"));
            };

        info!("Decoding {} ...", self.file_name());

        // Execute decoding
        decoding::decode(&self, &self.next_path()?, user, password)?;

        info!("Decoded {}", self.file_name());

        // Update video (status, path)
        self.change_to_next_status()?;

        Ok(())
    }

    // Move decoded video to archive directory
    fn move_to_archive_dir(&self) -> anyhow::Result<()> {
        // Nothing to do if video is not in status "decoded"
        if self.status() != Status::Decoded {
            return Ok(());
        }

        if let Err(err) = fs::rename(
            &self.p,
            dirs::working_sub_dir(&DirKind::Archive)
                .unwrap()
                .join(self.file_name()),
        ) {
            error!(
                "{:?}",
                anyhow!(err)
                    .context("Could not move video to archive directory after successful cutting")
            );
        }

        Ok(())
    }

    /// Move a video file to the working sub directory corresponding to the status
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
                .with_extension(format!(
                    "cut.{}",
                    self.p.extension().unwrap().to_str().unwrap()
                ))),
            _ => Ok(self.p.to_path_buf()),
        }
    }
}
