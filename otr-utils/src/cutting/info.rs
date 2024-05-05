use anyhow::{anyhow, Context};
use duct::cmd;
use log::*;
use scopeguard::defer;
use std::path::Path;
use std::{
    fmt,
    fs::{self, File},
    io::{BufRead, BufReader},
    process::Output,
    str::{from_utf8, FromStr},
};

use super::interval::{Frame, Time};

/// Extensions of FFMS2 index files
const FFMS2_INDEX_EXT: &str = "ffindex";
const FFMS2_TIMES_INDEX_EXT: &str = "ffindex_track00.tc.txt";
const FFMS2_KEY_FRAMES_INDEX_EXT: &str = "ffindex_track00.kf.txt";

/// Type of a stream
#[derive(Clone, Default, PartialEq)]
pub enum StreamType {
    Audio,
    Video,
    #[default]
    Unknown,
}
impl fmt::Display for StreamType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                StreamType::Audio => "audio",
                StreamType::Video => "video",
                StreamType::Unknown => "unknown",
            }
        )
    }
}
impl FromStr for StreamType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "audio" => StreamType::Audio,
            "video" => StreamType::Video,
            _ => StreamType::Unknown,
        })
    }
}

/// Alias for codec name
type Codec = Option<String>;

#[derive(Default)]
pub struct Stream {
    index: usize,
    codec: Codec,
}
impl Stream {
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn codec(&self) -> Codec {
        self.codec.clone()
    }
}

/// Metadata of a video file
pub struct Metadata {
    times: Vec<Time>,
    key_frames: Vec<Frame>,
    streams: Vec<Stream>,
}

impl Metadata {
    /// Create a new metadata object for a video
    pub fn new<P>(video: P) -> anyhow::Result<Self>
    where
        P: AsRef<Path>,
    {
        trace!("Retrieving metadata ...");

        // Retrieve stream metadata via ffprobe
        let mut streams: Vec<Stream> = vec![];
        match ffprobe::ffprobe(&video) {
            Ok(ffprobe_info) => {
                let mut first_audio_index: Option<usize> = None;
                let mut first_video_index: Option<usize> = None;

                for (i, s) in ffprobe_info.streams.iter().enumerate() {
                    let stream = Stream {
                        index: usize::try_from(s.index)?,
                        codec: s.codec_name.clone(),
                    };

                    // Verify data for audio streams
                    if let Some(typ) = &s.codec_type {
                        if StreamType::from_str(typ).unwrap() == StreamType::Audio {
                            if stream.codec.is_none() {
                                return Err(anyhow!(
                                    "Audio stream {} has no codec assigned",
                                    stream.index
                                ));
                            }
                            // Check if it is the first audio stream and remember its
                            // index for later use
                            if first_audio_index.is_none() {
                                first_audio_index = Some(i)
                            }
                        }
                    }

                    // Verify data for video streams
                    if let Some(typ) = &s.codec_type {
                        if StreamType::from_str(typ).unwrap() == StreamType::Video {
                            if stream.codec.is_none() {
                                return Err(anyhow!(
                                    "Video stream {} has no codec assigned",
                                    stream.index
                                ));
                            }
                            // Check if it is the first video stream and remember its
                            // index for later use
                            if first_video_index.is_none() {
                                first_video_index = Some(i)
                            }
                        }
                    }

                    streams.push(stream);
                }

                // Video must have at least one video or one audio stream
                if first_audio_index.is_none() && first_video_index.is_none() {
                    return Err(anyhow!("Video neither has a video nor an audio stream"));
                }
            }
            Err(err) => {
                return Err(anyhow!(
                    "Could not retrieve stream metadata with ffprobe: {:?}",
                    err
                ));
            }
        }

        let (times, key_frames) = retrieve_indexes(video)?;

        trace!("Metadata retrieved");

        Ok(Metadata {
            times,
            key_frames,
            streams,
        })
    }

    /// Returns an iterator for streams
    pub fn streams(&self) -> std::slice::Iter<'_, Stream> {
        self.streams.iter()
    }

    /// true if video has frames, false otherwise.
    /// Note: Pure audio files might not have frames
    pub fn has_frames(&self) -> bool {
        !self.times.is_empty()
    }

    pub fn frame_to_time(&self, frame: Frame) -> anyhow::Result<Time> {
        if frame >= Frame::from(self.times.len()) {
            Err(anyhow!(
                "Video does not contain a frame with number {}",
                frame
            ))
        } else {
            Ok(self.times[usize::from(frame)])
        }
    }

    /// Returns the number of the frame that corresponds to time. If there is no
    /// frame for that timestamp, the number of a frame that is close by is returned
    pub fn time_to_frame(&self, time: Time) -> anyhow::Result<Frame> {
        if self.times.is_empty() {
            return Err(anyhow!(
                "Cannot determine frame number of time {} since there are no frames",
                time
            ));
        }

        Ok(Frame::from(match self.times.binary_search(&time) {
            Ok(i) => i,
            Err(i) => {
                if i == self.times.len() {
                    i - 1
                } else {
                    i
                }
            }
        }))
    }

    /// Returns Some(frame) if frame is a key frame. Otherwise Some(n) is
    /// returned, where n is the number of the first key frame less than
    /// frame and where n >= limit. If such a key frame does not exist, none
    /// is returned
    pub fn key_frame_less_or_equal_until_limit(&self, frame: Frame, limit: Frame) -> Option<Frame> {
        if limit > frame {
            panic!("Limit must be less than or equal to frame")
        }

        match self.key_frames.binary_search(&frame) {
            Ok(_) => Some(frame),
            Err(i) => {
                if i > 0 && self.key_frames[i - 1] >= limit {
                    Some(self.key_frames[i - 1])
                } else {
                    None
                }
            }
        }
    }

    /// Returns Some(frame) if frame is a key frame. Otherwise Some(n) is
    /// returned, where n is the number of the first key frame greater than
    /// frame and where n <= limit. If such a key frame does not exist, none
    /// is returned
    pub fn key_frame_greater_or_equal_until_limit(
        &self,
        frame: Frame,
        limit: Frame,
    ) -> Option<Frame> {
        if limit < frame {
            panic!("Limit must be greater than or equal to frame")
        }

        match self.key_frames.binary_search(&frame) {
            Ok(_) => Some(frame),
            Err(i) => {
                if i + 1 < self.key_frames.len() && self.key_frames[i + 1] <= limit {
                    Some(self.key_frames[i + 1])
                } else {
                    None
                }
            }
        }
    }
}

/// Retrieves list of timestamps (i.e., the timestamps for each frames sorted
/// ascending) and the list of key frames sorted ascending by frame number.
/// This is done by calling ffmsindex
fn retrieve_indexes<P>(video: P) -> anyhow::Result<(Vec<Time>, Vec<Frame>)>
where
    P: AsRef<Path>,
{
    // Make sure FFMS2 index files are removed ultimately
    defer! {
    for ext in [FFMS2_INDEX_EXT,FFMS2_KEY_FRAMES_INDEX_EXT,FFMS2_TIMES_INDEX_EXT].iter() {
            _ = fs::remove_file(Path::new(&format!(
            "{}.{}",
            video.as_ref().display(),
            ext
        )));
    }
        trace!("Removed FFMS2 index files");
        }

    trace!("Retrieving data from FFMS2 index ...");

    let output: Output = cmd!("ffmsindex", "-f", "-k", "-c", video.as_ref().as_os_str(),)
        .stdout_null()
        .stderr_capture()
        .unchecked()
        .run()
        .context("Could not execute ffmsindex to create index files")?;

    if !output.status.success() {
        return Err(anyhow!("ffmsindex: {}", from_utf8(&output.stderr).unwrap())
            .context("Could not create key frame / time indexes"));
    }

    // Retrieve data from times index file
    let mut times: Vec<Time> = vec![];
    if let Ok(file) = File::open(Path::new(&format!(
        "{}.{}",
        video.as_ref().display(),
        FFMS2_TIMES_INDEX_EXT
    ))) {
        // Skip first line since it is a comment
        for line in BufReader::new(file).lines().skip(1).flatten() {
            // Times index contains time in milliseconds, but Time expects
            // seconds
            times.push(Time::from(
                &line
                    .parse::<f64>()
                    .context(format!("Could not convert \"{}\" into time", line))?
                    / 1000.0,
            ));
        }
    } else {
        debug!("Times index file does not exist");
    }

    // Retrieve data from key frames index file
    let mut key_frames: Vec<Frame> = vec![];
    if let Ok(file) = File::open(Path::new(&format!(
        "{}.{}",
        video.as_ref().display(),
        FFMS2_KEY_FRAMES_INDEX_EXT
    ))) {
        // Skip first 2 lines since they are comments / not relevant
        for line in BufReader::new(file).lines().skip(2).flatten() {
            key_frames.push(Frame::from_str(&line)?);
        }
    } else {
        debug!("Key frames index file does not exist");
    }

    trace!("Retrieved data from FFMS2 index");

    Ok((times, key_frames))
}
