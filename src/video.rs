use super::{cfg, cfg::DirKind};
use anyhow::{anyhow, Context};
use once_cell::sync::OnceCell;
use regex::Regex;
use std::{cmp, fmt, fs, path::Path, path::PathBuf};

/// Key of an OTR video. That's the left part of the file name ending with
/// "_TVOON_DE". I.e., key of
/// Blue_in_the_Face_-_Alles_blauer_Dunst_22.01.08_22-00_one_85_TVOON_DE.mpg.HD.avi
/// is
/// Blue_in_the_Face_-_Alles_blauer_Dunst_22.01.08_22-00_one_85_TVOON_DE
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd)]
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
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
#[derive(Clone, Debug)]
pub struct Video {
    p: PathBuf, // path
    k: Key,     // key
    s: Status,  // status
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

    // File name of a Video (i.e., the last part of its path)
    pub fn file_name(&self) -> &str {
        self.p.file_name().unwrap().to_str().unwrap()
    }

    // True if video already cut, false otherwise.
    pub fn is_processed(&self) -> bool {
        self.status() == Status::Cut
    }

    // Path of the video it would have if it had the next status - i.e., the
    // decoded status if it is encoded now or the cut status if it is decoded
    // now. If the video is already cut, its current path is returned.
    pub fn next_path(&self) -> anyhow::Result<PathBuf> {
        match self.s {
            Status::Encoded => Ok(self
                .as_ref()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(cfg::working_sub_dir(&(Status::Decoded).as_dir_kind())?)
                .join(self.file_name())
                .with_extension("")),
            Status::Decoded => Ok(self
                .as_ref()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(cfg::working_sub_dir(&(Status::Cut).as_dir_kind())?)
                .join(self.file_name())
                .with_extension(
                    "cut".to_string() + "." + self.as_ref().extension().unwrap().to_str().unwrap(),
                )),
            _ => Ok(self.p.to_path_buf()),
        }
    }

    // Changes the videos to the next status (i.e., if its in status encoded,
    // it is set to decoded, and if it is in status decoded it will be set to
    // cut). The video path is changed accordingly.
    pub fn change_to_next_status(&mut self) -> anyhow::Result<()> {
        if let Some(next_status) = self.s.next() {
            // NOTE: The new status must not we set before next_path() is
            //       executed first since next_path() uses the status !!!
            self.p = self.next_path()?;
            self.s = next_status;
        }
        Ok(())
    }
}

/// Support conversion of a &PathBuf into a Video. Usage of From trait is not
/// possible since not all paths represent OTR videos and thus, an error can
/// occur
impl TryFrom<&PathBuf> for Video {
    type Error = anyhow::Error;

    fn try_from(path: &PathBuf) -> Result<Self, Self::Error> {
        if let Some(file_name) = path.file_name() {
            if let Some(file_name_str) = file_name.to_str() {
                // check if path represents a cut video file (the check for cut
                // video files must be done before the check for uncut video
                // file since cut video files in some cases also match the
                // regex for uncut files)
                if regex_cut_video().is_match(file_name_str) {
                    // assemble Video instance
                    let captures = regex_cut_video().captures(file_name_str).unwrap();
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
                    });
                }
                // check if path represents an encoded or decoded video file
                if regex_uncut_video().is_match(file_name_str) {
                    // assemble Video instance
                    let captures = regex_uncut_video().captures(file_name_str).unwrap();
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
                    });
                }
            }
        }
        Err(anyhow!("{:?} is not a valid video file", path))
    }
}

/// Support usage of Video as &Path
impl AsRef<Path> for Video {
    fn as_ref(&self) -> &Path {
        &self.p
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

/// Collect video files from the working (sub) directories and from the paths
/// submitted via the command line, creates the corresponding Video instances
/// and returns them as vector, sorted by key (ascending) and status
/// (descending).
pub fn collect_and_sort() -> anyhow::Result<Vec<Video>> {
    // collect_videos_from_dir collects videos from the directory that is
    // assigned to kind dir_kind
    fn collect_videos_from_dir(dir_kind: &DirKind) -> anyhow::Result<Vec<Video>> {
        let mut videos: Vec<Video> = Vec::new();
        let dir = cfg::working_sub_dir(dir_kind)
            .context(format!("Could determine '{:?}' directory", &dir_kind))?;
        if !dir.is_dir() {
            return Err(anyhow!(format!("{:?} is not a directory: Ignored", dir)));
        }
        for file in fs::read_dir(dir)
            .with_context(|| format!("Could not read '{:?}' directory", &dir_kind))?
        {
            if !file.as_ref().unwrap().file_type().unwrap().is_file() {
                continue;
            }

            match Video::try_from(&file.as_ref().unwrap().path()) {
                Ok(video) => {
                    videos.push(video);
                }
                Err(_) => {
                    println!(
                        "{:?} is not a valid video file: Ignored",
                        &file.as_ref().unwrap().path()
                    );
                    continue;
                }
            }
        }
        Ok(videos)
    }

    let mut videos: Vec<Video> = Vec::new();

    // collect videos from command line parameters
    for path in cfg::videos() {
        if let Ok(video) = Video::try_from(path) {
            videos.push(video);
            continue;
        }
        println!("{:?} is not a valid video file: Ignored", path)
    }

    // if no videos have been submiited via command line: collect videos from
    // working (sub) directories
    if videos.is_empty() {
        for dir_kind in [
            DirKind::Root,
            DirKind::Encoded,
            DirKind::Decoded,
            DirKind::Cut,
        ] {
            videos.append(&mut collect_videos_from_dir(&dir_kind).context(format!(
                "Could not retrieve videos from '{:?}' sub directory",
                &dir_kind
            ))?);
        }
    }

    if videos.is_empty() {
        println!("No videos found :(");
    } else {
        videos.sort();
    }

    Ok(videos)
}

/// Moves a video file to the working sub directory corresponding to the status
/// of the video. The Video (i.e., its path) is changed accordingly.
pub fn move_to_working_dir(video: &mut Video) -> Option<anyhow::Error> {
    // since video path was already checked for compliance before, it is OK to
    // simply unwrap the result
    let source_dir = video.as_ref().parent().unwrap();

    let target_dir = cfg::working_sub_dir(&(video.status()).as_dir_kind()).ok()?;
    let target_path = target_dir.join(video.file_name());

    // nothing to do if video is already in correct directory
    if source_dir == target_dir {
        return None;
    }

    // copy video file to working sub directory and adjust path
    fs::rename(video.as_ref(), &target_path).ok()?;
    video.p = target_path.to_path_buf();

    None
}

/// Regular expression to analyze the name of a (potential) video file that is
/// not cut - i.e., either encoded or decoded.
fn regex_uncut_video() -> &'static Regex {
    static RE_VALID_VIDEO: OnceCell<Regex> = OnceCell::new();
    RE_VALID_VIDEO.get_or_init(|| {
        Regex::new(r"^([^\.]+_\d{2}.\d{2}.\d{2}_\d{2}-\d{2}_[^_]+_\d+_TVOON_DE)\.[^\.]+(?P<fmt>\.(HQ|HD))?(?P<ext>\.[^\.]+)(?P<encext>\.otrkey)?$").unwrap()
    })
}

/// Regular expression to analyze the name of a (potential) video file that is
/// cut
fn regex_cut_video() -> &'static Regex {
    static RE_VALID_VIDEO: OnceCell<Regex> = OnceCell::new();
    RE_VALID_VIDEO.get_or_init(|| {
        Regex::new(r"^([^\.]+_\d{2}.\d{2}.\d{2}_\d{2}-\d{2}_[^_]+_\d+_TVOON_DE)\.(.*cut\..+)$")
            .unwrap()
    })
}
