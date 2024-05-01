use anyhow::anyhow;
use log::*;
use std::path::Path;
use std::{fmt, str::FromStr, sync::Once};

use super::interval::{Frame, Time};

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

        // Create FFMS2 index for video
        // FFMS2 must be initialized before it can be used
        init_ffms2();
        let ffms2_indexer = ffms2::index::Indexer::new(video.as_ref());
        if let Err(err) = ffms2_indexer {
            return Err(anyhow!("{:?}", err).context(format!(
                "Could not create FFMS2 indexer for video {}",
                video.as_ref().display()
            )));
        }
        let ffms2_index = ffms2_indexer
            .unwrap()
            .DoIndexing2(ffms2::IndexErrorHandling::IEH_ABORT);
        if let Err(err) = ffms2_index {
            return Err(anyhow!("{:?}", err).context(format!(
                "Could not create FFMS2 index for video {}",
                video.as_ref().display()
            )));
        }
        let ffms2_index = ffms2_index.unwrap();
        trace!("FFMS2 index created");

        let mut streams: Vec<Stream> = vec![];
        let mut times: Vec<Time> = vec![];
        let mut key_frames: Vec<Frame> = vec![];

        let mut main_stream: Option<ffms2::track::Track> = None;

        // Retrieve stream metadata via ffprobe
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

                    // Set main stream. The "main stream" is the stream that is
                    // used for mappings between timestamps and frame numbers.
                    // If a file has video streams, the first of these will be
                    // used as main stream. Otherwise, if the file has audio
                    // streams, the first of these is used. If the file neither
                    // has video nor audio streams, an error is returned
                    let main_index: usize;
                    if let Some(index) = first_video_index {
                        main_index = index;
                    } else if let Some(index) = first_audio_index {
                        main_index = index;
                    } else {
                        return Err(anyhow!("Cannot determine \"main stream\", since video neither has a video nor an audio stream"));
                    }
                    main_stream = Some(ffms2::track::Track::TrackFromIndex(
                        &ffms2_index,
                        main_index,
                    ));
                }
            }
            Err(err) => {
                return Err(anyhow!(
                    "Could not retrieve stream metadata with ffprobe: {:?}",
                    err
                ));
            }
        }

        // Create (a) index for mapping timestamps to frame numbers
        //        (b) key_frames array
        if let Some(stream) = main_stream {
            for i in 0..stream.NumFrames() {
                times.push(
                    // Calculate time. Formula taken from:
                    // https://github.com/FFMS/ffms2/blob/master/doc/ffms2-api.md#ffms_frameinfo
                    Time::from(
                        stream.FrameInfo(i).PTS as f64 * stream.TimeBase().Num as f64
                            / stream.TimeBase().Den as f64
                            / 1000_f64,
                    ),
                );
                if stream.FrameInfo(i).KeyFrame == 1 {
                    key_frames.push(Frame::from(i));
                }
            }
        } else {
            return Err(anyhow!(
                "Could not determine \"main stream\", since video might have no streams at all"
            ));
        }

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

/// Make sure that ffms2 initilization is only done once
static INIT: Once = Once::new();
fn init_ffms2() {
    INIT.call_once(|| {
        ffms2::FFMS2::Init();
        trace!("FFMS2 initialized");
    });
}
