use anyhow::{anyhow, Context};
use duct::cmd;
use log::*;
use std::path::Path;
use std::{fmt, sync::Once};

/// Type aliases for having semantically clear function interfaces
type Timestamp = u64;
type FrameNo = u64;

/// Support mapping from timestamp to frame number
struct Time2FrameNo {
    time: Timestamp,
    frame_no: FrameNo,
}

/// Type of track
pub enum TrackKind {
    Audio,
    Video,
    Unknown,
}
impl fmt::Display for TrackKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TrackKind::Audio => write!(f, "audio"),
            TrackKind::Video => write!(f, "video"),
            TrackKind::Unknown => write!(f, "unknown"),
        }
    }
}

/// Codecs
pub enum Codec {
    AC3,
    MP3,
    H264,
    MPEG4,
    Unknown,
}
impl fmt::Display for Codec {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Codec::AC3 => write!(f, "AC3"),
            Codec::MP3 => write!(f, "MP3"),
            Codec::H264 => write!(f, "H.264"),
            Codec::MPEG4 => write!(f, "MPEG-4"),
            Codec::Unknown => write!(f, "unknown"),
        }
    }
}

/// Metadata of a video file
pub struct Metadata {
    ffprobe_data: FFProbeData,
    ffms2_index: ffms2::index::Index,
    time2frame: Vec<Time2FrameNo>,
}

impl Metadata {
    /// Create a new metadata object for a video
    pub fn new<P>(path: P) -> anyhow::Result<Self>
    where
        P: AsRef<Path>,
    {
        trace!(
            "Retrieving metadata for video {} ...",
            path.as_ref().display()
        );

        // Create FFMS2 index for video
        // FFMS2 must be initialized before it can be used
        init_ffms2();
        let ffms2_indexer = ffms2::index::Indexer::new(path.as_ref());
        if let Err(err) = ffms2_indexer {
            return Err(anyhow!("{:?}", err).context(format!(
                "Could not create FFMS2 indexer for video {}",
                path.as_ref().display()
            )));
        }
        let ffms2_index = ffms2_indexer
            .unwrap()
            .DoIndexing2(ffms2::IndexErrorHandling::IEH_ABORT);
        if let Err(err) = ffms2_index {
            return Err(anyhow!("{:?}", err).context(format!(
                "Could not create FFMS2 index for video {}",
                path.as_ref().display()
            )));
        }
        let ffms2_index = ffms2_index.unwrap();
        trace!("FFMS2 index created");

        let mut metadata = Metadata {
            ffms2_index,
            // Retrieve additional data via ffprobe
            ffprobe_data: FFProbeData::new(&path)?,
            time2frame: vec![],
        };
        trace!("Data via ffprobe retrieved");

        // Create index for mapping timestamps to frame numbers
        let track_no = metadata
            .leading_track()
            .context("Cannot create index for mapping from timestamps to frame numbers")?;
        for frame_no in 0..metadata.track(track_no).num_frames() {
            metadata.time2frame.push(Time2FrameNo {
                time: metadata.track(track_no).frame(frame_no).timestamp(),
                frame_no,
            });
        }
        metadata.time2frame.sort_by_key(|a| a.time);
        trace!("Time->frame index created");

        trace!("Metadata for video {} retrieved", path.as_ref().display());

        Ok(metadata)
    }

    /// Returns the number of tracks/streams
    pub fn num_tracks(&self) -> usize {
        self.ffms2_index.NumTracks()
    }

    /// Track of index track_no. Is there is no track with that index, the
    /// function panics
    pub fn track(&self, track_no: usize) -> Track {
        if track_no >= self.num_tracks() {
            panic!("Video does not have a track no {}", track_no);
        }

        Track {
            ffms2_track: ffms2::track::Track::TrackFromIndex(&self.ffms2_index, track_no),
            raw_track: &self.ffprobe_data.tracks[track_no],
        }
    }

    /// Returns the number of the frame that corresponds to timestamp time. If
    /// there is no frame for that timestamp, the number of frame that is close
    /// by is returned
    pub fn frame_no_from_timestamp(&self, time: Timestamp) -> anyhow::Result<FrameNo> {
        if self.time2frame.is_empty() {
            return Err(anyhow!(
                "Cannot determine frame number of timestamp {} since there are no frames",
                time
            ));
        }

        Ok(
            match self.time2frame.binary_search_by_key(&time, |a| a.time) {
                Ok(i) => self.time2frame[i].frame_no,
                Err(i) => {
                    if i == self.time2frame.len() {
                        self.time2frame.last().unwrap().frame_no
                    } else {
                        self.time2frame[i].frame_no
                    }
                }
            },
        )
    }

    /// Returns the number of the "leading track" - i.e., the track that is used
    /// for mappings between timestamps and frame numbers.
    /// If a file has video tracks, the first of these will be used as leading
    /// track. Otherwise, if the file has audio tracks, the first of these is
    /// used. If the file neither has video nor audio tracks, an error is
    /// returned
    fn leading_track(&self) -> anyhow::Result<usize> {
        if let Ok(track_no) = self
            .ffms2_index
            .FirstTrackOfType(ffms2::track::TrackType::TYPE_VIDEO)
        {
            trace!("Creating time->frame index based on video track ...");
            return Ok(track_no);
        }
        if let Ok(track_no) = self
            .ffms2_index
            .FirstTrackOfType(ffms2::track::TrackType::TYPE_AUDIO)
        {
            trace!("Creating time->frame index based on audio track");
            return Ok(track_no);
        }

        Err(anyhow!("Video neither has a video nor an audio track ..."))
    }
}

/// Track data. Combines data from the FFMS2 index and data retrieved via ffprobe
pub struct Track<'a> {
    ffms2_track: ffms2::track::Track,
    raw_track: &'a FFProbeTrack,
}
impl Track<'_> {
    /// Codec of the track
    pub fn codec(&self) -> Codec {
        match self.raw_track.codec_name.as_str() {
            "ac3" => Codec::AC3,
            "mp3" => Codec::MP3,
            "h264" => Codec::H264,
            "mpeg4" => Codec::MPEG4,
            _ => Codec::Unknown,
        }
    }

    /// Type of the track (whether it's a video or audio track or anything else)
    pub fn kind(&self) -> TrackKind {
        match self.raw_track.codec_type.as_str() {
            "audio" => TrackKind::Audio,
            "video" => TrackKind::Video,
            _ => TrackKind::Unknown,
        }
    }

    /// Time base of a track
    pub fn time_base(&self) -> f64 {
        self.ffms2_track.TimeBase().Num as f64 / self.ffms2_track.TimeBase().Den as f64
    }

    /// Number of frames of a track
    pub fn num_frames(&self) -> FrameNo {
        self.ffms2_track.NumFrames() as FrameNo
    }

    /// Returns frame number frame_no
    pub fn frame(&self, frame_no: FrameNo) -> Frame {
        Frame {
            track: self,
            ffms2_frame_info: self.ffms2_track.FrameInfo(frame_no as usize),
        }
    }
}

/// Frame data. Combines data from the FFMS2 index and data retrieved via ffprobe
pub struct Frame<'a> {
    track: &'a Track<'a>,
    ffms2_frame_info: ffms2::frame::FrameInfo,
}
impl Frame<'_> {
    /// Whether it is a key frame or not
    pub fn is_key_frame(&self) -> bool {
        self.ffms2_frame_info.KeyFrame == 1
    }

    /// Tiemstamp of the frame
    pub fn timestamp(&self) -> Timestamp {
        (self.ffms2_frame_info.PTS as f64 * self.track.time_base() * 1000.0) as Timestamp
    }
}

/// Structure to hold data retrieved with ffprobe
#[derive(serde::Deserialize, Debug, Default)]
struct FFProbeData {
    format: FFProbeFormat,
    #[serde(rename = "streams")]
    tracks: Vec<FFProbeTrack>,
}
#[derive(serde::Deserialize, Debug, Default)]
struct FFProbeFormat {
    #[serde(rename = "nb_streams")]
    num_tracks: usize,
    format_name: String,
    bit_rate: String,
}
#[derive(serde::Deserialize, Debug, Default)]
struct FFProbeTrack {
    #[serde(rename = "index")]
    track_id: usize,
    codec_name: String,
    codec_type: String,
    bit_rate: String,
}
impl FFProbeData {
    /// Retrieve metadata of a video file with ffprobe
    fn new<P>(path: P) -> anyhow::Result<Self>
    where
        P: AsRef<Path>,
    {
        serde_json::from_str(
            &cmd!(
                "ffprobe",
                "-loglevel",
                "0",
                "-print_format",
                "json",
                "-show_format",
                "-show_streams",
                path.as_ref().as_os_str()
            )
            .stdout_capture()
            .unchecked()
            .read()
            .with_context(|| {
                format!(
                    "Could not read metadata of video {}",
                    path.as_ref().display()
                )
            })?,
        )
        .with_context(|| "Cannot read metadata")
    }
}

/// Make sure that ffms2 initilization is only done once
static INIT: Once = Once::new();
fn init_ffms2() {
    INIT.call_once(|| {
        ffms2::FFMS2::Init();
        trace!("FFMS2 initialized");
    });
}
