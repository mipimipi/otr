use super::{
    cfg,
    video::{Status, Video},
};
use anyhow::{anyhow, Context};
use ini::Ini;
use serde::Deserialize;
use std::{cmp, error::Error, fmt, fmt::Debug, fmt::Write, fs, path::Path, process::Command, str};

/// URI's for the retrieval of cutlist data
const CUTLIST_RETRIEVE_HEADERS_URI: &str = "http://cutlist.at/getxml.php?name=";
const CUTLIST_RETRIEVE_LIST_DETAILS_URI: &str = "http://cutlist.at/getfile.php?id=";

/// names for sections and attributs for INI file
const CUTLIST_ITEM_GENERAL_SECTION: &str = "General";
const CUTLIST_ITEM_NUM_OF_CUTS: &str = "NoOfCuts";
const CUTLIST_ITEM_CUT_SECTION: &str = "Cut";
const CUTLIST_ITEM_TIMES_START: &str = "Start";
const CUTLIST_ITEM_TIMES_DURATION: &str = "Duration";
const CUTLIST_ITEM_FRAMES_START: &str = "StartFrame";
const CUTLIST_ITEM_FRAMES_DURATION: &str = "DurationFrames";

/// special error type for cutting videos to be able to handle specific
/// situations - e.g., if no cutlist exists
#[derive(Debug)]
pub enum CutError {
    Any(anyhow::Error),
    NoCutlist,
}
// support the use of "{}" format specifier
impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CutError::Any(ref source) => write!(f, "Error: {}", source),
            CutError::NoCutlist => write!(f, "No cutlist exists"),
        }
    }
}
// support conversion an Error into CutError
impl Error for CutError {}
// support conversion of an anyhow::Error into CutError
impl From<anyhow::Error> for CutError {
    fn from(err: anyhow::Error) -> CutError {
        CutError::Any(err)
    }
}

/// cut cuts a decoded Video and returns the cut Video
pub fn cut(dec_video: &Video) -> Result<Video, CutError> {
    // nothing to do if dec_video is not in status "decoded"
    if dec_video.status() != Status::Decoded {
        return Ok(dec_video.clone());
    }

    println!("Cutting {:?} ...", dec_video.file_name());

    let cut_video = Video::new_cut_from_decoded(dec_video).unwrap();

    // retrieve cutlist headers
    let headers: Vec<CutlistHeader> = match cutlist_headers(dec_video).context(format!(
        "Could not retrieve cutlists for {:?}",
        dec_video.file_name()
    )) {
        Ok(hdrs) => hdrs,
        _ => return Err(CutError::NoCutlist),
    };

    // retrieve cutlists and cut video
    let mut is_cut = false;
    for header in headers {
        match cutlist(&header) {
            Ok(items) => {
                // cut video with mkvmerge
                match cut_with_mkvmerge(dec_video.as_ref(), cut_video.as_ref(), &header, &items) {
                    Ok(_) => {
                        // exit loop since video is cut
                        is_cut = true;
                        break;
                    }
                    Err(err) => {
                        eprintln!(
                            "{:?}",
                            anyhow!(err).context(format!(
                                "Could not cut {:?} with cutlist {}",
                                dec_video.file_name(),
                                header.id
                            ))
                        );
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "{:?}",
                    anyhow!(err).context(format!(
                        "Could not retrieve cutlist {} for {:?}",
                        header.id,
                        dec_video.file_name()
                    ))
                );
            }
        }
    }

    // in case of having cut the video successfully, move decoded video to
    // archive directory and return with OK. Otherwise return with error
    if is_cut {
        if let Err(err) = fs::rename(
            dec_video.as_ref(),
            cfg::working_sub_dir(&cfg::DirKind::Archive)
                .unwrap()
                .join(dec_video.file_name()),
        ) {
            eprintln!(
                "{:?}",
                anyhow!(err).context(format!(
                    "Could not move {:?} to archive directory after successful cutting",
                    dec_video.file_name()
                ))
            );
        }
        println!("Cut {:?}", dec_video.file_name());
        Ok(cut_video)
    } else {
        Err(CutError::Any(anyhow!(
            "No cutlist could be successfully applied to cut {:?}",
            dec_video.file_name()
        )))
    }
}

/// CutKind represents the kind of a cut - i.e., whether it is expressed in
/// frame numbers or times
#[derive(Debug)]
enum CutKind {
    Frames,
    Times,
}

/// CutlistHeader represents the header of a cutlist
#[derive(Debug)]
struct CutlistHeader {
    id: u64,
    rating: f64,
    kind: CutKind,
}
impl Eq for CutlistHeader {}
impl Ord for CutlistHeader {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self.rating < other.rating {
            return cmp::Ordering::Less;
        };
        if self.rating > other.rating {
            return cmp::Ordering::Greater;
        };
        cmp::Ordering::Equal
    }
}
impl PartialEq for CutlistHeader {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl PartialOrd for CutlistHeader {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// cutlist_headers retrieves the headers of potentially existing cutlists for
/// a Video. If no cutlists exists, an empty array but no error is returned.
fn cutlist_headers(video: &Video) -> anyhow::Result<Vec<CutlistHeader>> {
    #[derive(Debug, Deserialize)]
    struct Headers {
        #[serde(rename = "cutlist")]
        headers: Vec<Header>,
    }
    #[derive(Debug, Deserialize)]
    struct Header {
        id: u64,
        rating: String,
        #[serde(rename = "withframes")]
        with_frames: String,
        errors: String,
    }

    let file_name = video.file_name();

    let response = reqwest::blocking::get(CUTLIST_RETRIEVE_HEADERS_URI.to_string() + file_name)
        .with_context(|| {
            format!(
                "Did not get a response for cutlist header request for {}",
                file_name
            )
        })?
        .text()
        .with_context(|| format!("Could not parse cutlist header response for {}", file_name))?;

    if response.is_empty() {
        return Err(anyhow!(format!("Did not find cutlist for {:?}", file_name)));
    }

    let mut headers: Vec<CutlistHeader> = vec![];

    let raw_headers: Headers = quick_xml::de::from_str(&response)
        .with_context(|| format!("Could not parse cutlist headers for {:?}", file_name))?;

    for raw_header in raw_headers.headers {
        // don't accept cutlists with errors
        let num_errs = raw_header.errors.parse::<i32>();
        if num_errs.is_err() || num_errs.unwrap() > 0 {
            continue;
        }

        // create default cutlist header
        let mut header: CutlistHeader = CutlistHeader {
            id: raw_header.id,
            rating: 0.0,
            kind: CutKind::Frames,
        };

        // parse rating
        if let Ok(rating) = raw_header.rating.parse::<f64>() {
            header.rating = rating;
        }

        // parse frames indicator
        if let Ok(with_frames) = raw_header.with_frames.parse::<i32>() {
            header.kind = if with_frames == 1 {
                CutKind::Frames
            } else {
                CutKind::Times
            };
        }

        headers.push(header);
    }

    headers.sort();
    Ok(headers)
}

/// CutPoint represents a start or end point of a cut
#[derive(Debug)]
pub enum CutPoint {
    Frame(f64),
    Time(f64),
}
impl fmt::Display for CutPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            CutPoint::Frame(point) => write!(f, "{:.0}", point),
            CutPoint::Time(point) => {
                let time: u64 = (point * 1000000_f64) as u64;
                let (seconds, subs) = (time / 1000000, time % 1000000);
                let (hours, rest) = (seconds / 3600, seconds % 3600);
                let (mins, rest) = (rest / 60, rest % 60);
                write!(f, "{:02}:{:02}:{:02}.{:06}", hours, mins, rest, subs)
            }
        }
    }
}

/// CutlistItem represents a cut of a cutlist - i.e., a start and an end point
#[derive(Debug)]
struct CutlistItem {
    start: CutPoint,
    end: CutPoint,
}
impl CutlistItem {
    // new creates a new CutListItem from a start point, a duration and
    // the kind of the cut
    fn new(start: &str, duration: &str, kind: &CutKind) -> anyhow::Result<Option<CutlistItem>> {
        // convert start and duration to floating point
        let start_f = start
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", start))?;
        let duration_f = duration
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", duration))?;

        // cutlist item with zero duration (i.e., equal start and end make no sense)
        if duration_f > 0.0 {
            // assemble cutlist item
            Ok(match kind {
                CutKind::Frames => Some(CutlistItem {
                    start: CutPoint::Frame(start_f),
                    end: CutPoint::Frame(start_f + duration_f),
                }),
                _ => Some(CutlistItem {
                    start: CutPoint::Time(start_f),
                    end: CutPoint::Time(start_f + duration_f),
                }),
            })
        } else {
            Ok(None)
        }
    }
}

/// cutlist_item_attr_start returns the attribute name for start of a cut
/// depending on the kind of the cut
fn cutlist_item_attr_start(kind: &CutKind) -> String {
    match kind {
        CutKind::Frames => CUTLIST_ITEM_FRAMES_START.to_string(),
        _ => CUTLIST_ITEM_TIMES_START.to_string(),
    }
}

/// cutlist_item_attr_duration returns the attribute name for the duration of a
/// cur depending the kind of the cut
fn cutlist_item_attr_duration(kind: &CutKind) -> String {
    match kind {
        CutKind::Frames => CUTLIST_ITEM_FRAMES_DURATION.to_string(),
        _ => CUTLIST_ITEM_TIMES_DURATION.to_string(),
    }
}

/// cutlist retrieves the cutlist (i.e., the different cuts) for a given cutlist
/// header
fn cutlist(header: &CutlistHeader) -> anyhow::Result<Vec<CutlistItem>> {
    // retrieve cutlist in INI format
    let response = reqwest::blocking::get(
        CUTLIST_RETRIEVE_LIST_DETAILS_URI.to_string() + &header.id.to_string(),
    )
    .with_context(|| {
        format!(
            "Did not get a response for requesting cutlist {}",
            header.id
        )
    })?
    .text()
    .with_context(|| format!("Could not parse response for cutlist {} as text", header.id))?;
    let list = Ini::load_from_str(&response)
        .with_context(|| format!("Could not parse response for cutlist {} as INI", header.id))?;

    // get number of cuts
    let num_cuts = list
        .section(Some(CUTLIST_ITEM_GENERAL_SECTION))
        .with_context(|| {
            format!(
                "Could not find section '{}' in cutlist {}",
                CUTLIST_ITEM_GENERAL_SECTION, header.id
            )
        })?
        .get(CUTLIST_ITEM_NUM_OF_CUTS)
        .with_context(|| {
            format!(
                "Could not find attribute '{}' in cutlist {}",
                CUTLIST_ITEM_NUM_OF_CUTS, header.id
            )
        })?
        .parse::<i32>()
        .with_context(|| {
            format!(
                "Could not parse attribute '{}' in cutlist {}",
                CUTLIST_ITEM_NUM_OF_CUTS, header.id
            )
        })?;

    // get cuts and create cutlist items from them
    let mut items: Vec<CutlistItem> = vec![];
    for i in 0..num_cuts {
        let cut = list
            .section(Some(format!("{}{}", CUTLIST_ITEM_CUT_SECTION, i)))
            .with_context(|| {
                format!(
                    "Could not find section for cut no {} in cutlist {}",
                    i, header.id
                )
            })?;
        if let Some(item) = CutlistItem::new(
            cut.get(cutlist_item_attr_start(&header.kind))
                .with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        cutlist_item_attr_start(&header.kind),
                        i
                    )
                })?,
            cut.get(cutlist_item_attr_duration(&header.kind))
                .with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        cutlist_item_attr_duration(&header.kind),
                        i
                    )
                })?,
            &header.kind,
        )? {
            items.push(item);
        }
    }

    Ok(items)
}

/// cut_with_mkvmerge cuts a video file stored in in_path with mkvmerge using
/// the cutlist information in header and items and stores the cut video in
/// out_path
fn cut_with_mkvmerge(
    in_path: &Path,
    out_path: &Path,
    header: &CutlistHeader,
    items: &[CutlistItem],
) -> anyhow::Result<()> {
    // assemble split parameter for mkvmerge
    let mut split_str = "".to_string();
    match header.kind {
        CutKind::Frames => split_str += "parts-frames:",
        _ => split_str += "parts:",
    }
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            split_str += ",+"
        }
        write!(split_str, "{}-{}", &item.start, &item.end).unwrap();
    }

    // call mkvmerge to cut the video
    let output = Command::new("mkvmerge")
        .arg("-o")
        .arg(out_path.to_str().unwrap())
        .arg("--split")
        .arg(split_str)
        .arg(in_path.to_str().unwrap())
        .output()
        .unwrap();
    if !output.status.success() {
        return Err(anyhow!(str::from_utf8(&output.stdout).unwrap().to_string())
            .context("mkvmerge returned an error"));
    }

    Ok(())
}
