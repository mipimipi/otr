use super::{cfg, cfg::DirKind};
use anyhow::{anyhow, Context};
use once_cell::sync::OnceCell;
use regex::Regex;
use std::{cmp, fs, path::Path, path::PathBuf};

/// Key represents the key of an OTR video, that's the left part of the file
/// name ending with "_TVOON_DE". I.e., key of
/// Blue_in_the_Face_-_Alles_blauer_Dunst_22.01.08_22-00_one_85_TVOON_DE.mpg.HD.avi
/// is
/// Blue_in_the_Face_-_Alles_blauer_Dunst_22.01.08_22-00_one_85_TVOON_DE
#[derive(Clone, Debug, Eq, Hash, PartialEq, PartialOrd)]
pub struct Key(String);
/// support conversion from &str to Key
impl From<&str> for Key {
    fn from(s: &str) -> Self {
        Key(s.to_string())
    }
}
/// support conversion from String to Key
impl From<String> for Key {
    fn from(s: String) -> Self {
        Key(s)
    }
}

/// Status represents the status of a video - i.e., whether its encoded,
/// decoded or cut. The status can be ordered: Encoded < Decoded < Cut
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
    fn to_dir_kind(self) -> DirKind {
        match self {
            Status::Encoded => DirKind::Encoded,
            Status::Decoded => DirKind::Decoded,
            Status::Cut => DirKind::Cut,
        }
    }
}

/// Video represents a video file downloaded from OTR, incl. its path, key and
/// status
#[derive(Clone, Debug)]
pub struct Video {
    p: PathBuf, // path
    k: Key,     // key
    s: Status,  // status
}
/// Support iteration over videos: Encoded video -> decoded video -> cut video
/// -> None
impl Iterator for Video {
    type Item = Video;

    // In case self is of status Encoded or Decoded, a new video of with the
    // same key, the following status (i.e., Decoded or Cut) and the
    // corresponding path is created. Otherwise (i.e.,  self is of status Cut)
    // None is returned
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(next_status) = self.s.next() {
            let video = Video {
                k: self.k.clone(),
                s: next_status,
                p: match self.s {
                    Status::Encoded => self
                        .as_ref()
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .join(cfg::working_sub_dir(&(next_status).to_dir_kind()).unwrap())
                        .join(self.file_name())
                        .with_extension(""),
                    Status::Decoded => self
                        .as_ref()
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .join(cfg::working_sub_dir(&(next_status).to_dir_kind()).unwrap())
                        .join(self.file_name())
                        .with_extension(
                            "cut".to_string()
                                + "."
                                + self.as_ref().extension().unwrap().to_str().unwrap(),
                        ),
                    _ => todo!(),
                },
            };
            return Some(video);
        }
        None
    }
}

impl Video {
    // key returns the key of a Video
    pub fn key(&self) -> &Key {
        &self.k
    }

    // status returns the status of a Video
    pub fn status(&self) -> Status {
        self.s
    }

    // file_name returns the file name of a Video
    pub fn file_name(&self) -> &str {
        self.p.file_name().unwrap().to_str().unwrap()
    }
}

/// support conversion of a &PathBuf into a Video. Usage of From trait is not
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

/// support usage of Video as &Path
impl AsRef<Path> for Video {
    fn as_ref(&self) -> &Path {
        &self.p
    }
}

/// support ordering of videos: By key (ascending), status (descending)
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

/// collect_and_sort collects video files from the working (sub) directories and
/// from the paths submitted via the command line, creates the corresponding
/// Video instances and returns them as vector, sorted by key (ascending) and
/// status (descending)
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

/// move_to_working_dir moves a Video to the working sub directory corresponding
/// to its status. The Video (i.e., its path) is changed accordingly
pub fn move_to_working_dir(video: &mut Video) -> Option<anyhow::Error> {
    // since video path was already checked for compliance before, it is OK to
    // simply unwrap the result
    let source_dir = video.as_ref().parent().unwrap();

    let target_dir = cfg::working_sub_dir(&(video.status()).to_dir_kind()).ok()?;
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

// Check if the video is alread cut. If that's the case, print a message.
// Other do nothing.
pub fn nothing_to_do(video: &Video) {
    if video.status() == Status::Cut {
        println!("Cut already: {:?}", video.file_name());
    }
}

/// regex_uncut_video returns a regular expression to analyze the name of a
/// (potential) video file that is not cut - i.e., either encoded or decoded
fn regex_uncut_video() -> &'static Regex {
    static RE_VALID_VIDEO: OnceCell<Regex> = OnceCell::new();
    RE_VALID_VIDEO.get_or_init(|| {
        Regex::new(r"^([^\.]+_\d{2}.\d{2}.\d{2}_\d{2}-\d{2}_[^_]+_\d+_TVOON_DE)\.[^\.]+(?P<fmt>\.(HQ|HD))?(?P<ext>\.[^\.]+)(?P<encext>\.otrkey)?$").unwrap()
    })
}

/// regex_cut_video returns a regular expression to analyze the name of a
/// (potential) video file that is cut
fn regex_cut_video() -> &'static Regex {
    static RE_VALID_VIDEO: OnceCell<Regex> = OnceCell::new();
    RE_VALID_VIDEO.get_or_init(|| {
        Regex::new(r"^([^\.]+_\d{2}.\d{2}.\d{2}_\d{2}-\d{2}_[^_]+_\d+_TVOON_DE)\.(.*cut\..+)$")
            .unwrap()
    })
}
