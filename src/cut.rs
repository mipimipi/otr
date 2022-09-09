use super::{
    cfg,
    video::{Status, Video},
};
use anyhow::{anyhow, Context};
use ini::Ini;
use serde::Deserialize;
use std::{cmp, error::Error, fmt, fmt::Debug, fmt::Write, fs, path::Path, process::Command, str};

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
// allow the use of "{}" format specifier
impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CutError::Any(ref source) => write!(f, "Error: {}", source),
            CutError::NoCutlist => write!(f, "No cutlist exists"),
        }
    }
}
// allow this type to be treated like an error
impl Error for CutError {}
// support converting anyhow::Error into CutError
impl From<anyhow::Error> for CutError {
    fn from(err: anyhow::Error) -> CutError {
        CutError::Any(err)
    }
}

/// cuts decoded dec_video and returns the cut video
pub fn cut(dec_video: &Video) -> Result<Video, CutError> {
    // nothing to do if dec_video is not in status "decoded"
    if dec_video.status() != Status::Decoded {
        return Ok(dec_video.clone());
    }

    println!("Cutting {:?} ...", dec_video.file_name());

    let cut_video = Video::new_cut_from_decoded(dec_video).unwrap();

    // retrieve cutlist headers
    let headers: Vec<CutlistHeader> = match cutlist_headers(dec_video.file_name()).context(format!(
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

#[derive(Debug)]
struct CutlistHeader {
    id: u64,
    rating: f64,
    with_frames: bool,
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

fn cutlist_headers(name: &str) -> anyhow::Result<Vec<CutlistHeader>> {
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

    let response = reqwest::blocking::get(CUTLIST_RETRIEVE_HEADERS_URI.to_string() + name)
        .with_context(|| {
            format!(
                "Did not get a response for cutlist header request for {}",
                name
            )
        })?
        .text()
        .with_context(|| format!("Could not parse cutlist header response for {}", name))?;

    if response.is_empty() {
        return Err(anyhow!(format!("Did not find cutlist for {:?}", name)));
    }

    let mut headers: Vec<CutlistHeader> = vec![];

    let raw_headers: Headers = quick_xml::de::from_str(&response)
        .with_context(|| format!("Could not parse cutlist headers for {:?}", name))?;

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
            with_frames: false,
        };

        // parse rating
        if let Ok(rating) = raw_header.rating.parse::<f64>() {
            header.rating = rating;
        }

        // parse frames indicator
        if let Ok(with_frames) = raw_header.with_frames.parse::<i32>() {
            header.with_frames = with_frames == 1;
        }

        headers.push(header);
    }

    headers.sort();
    Ok(headers)
}

#[derive(Debug, Default)]
struct CutlistItem {
    start: String,
    end: String,
}
impl CutlistItem {
    fn new(
        start: &str,
        duration: &str,
        convert: fn(f64) -> String,
    ) -> anyhow::Result<Option<CutlistItem>> {
        // convert start and duration to floating point
        let start_f = start
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", start))?;
        let duration_f = duration
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", duration))?;

        // cutlist item withs zero duration (i.e., equal start and end make no sense)
        if duration_f > 0.0 {
            // assemble cutlist item
            Ok(Some(CutlistItem {
                start: convert(start_f),
                end: convert(start_f + duration_f),
            }))
        } else {
            Ok(None)
        }
    }

    fn convert_frame(f: f64) -> String {
        format!("{:.0}", f)
    }
    fn convert_time(f: f64) -> String {
        let time: u64 = (f * 1000000_f64) as u64;
        let (seconds, subs) = (time / 1000000, time % 1000000);
        let (hours, rest) = (seconds / 3600, seconds % 3600);
        let (mins, rest) = (rest / 60, rest % 60);
        format!("{:02}:{:02}:{:02}.{:06}", hours, mins, rest, subs)
    }
}

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

        if let Some(item) = if !header.with_frames {
            CutlistItem::new(
                cut.get(CUTLIST_ITEM_TIMES_START).with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        CUTLIST_ITEM_TIMES_START, i
                    )
                })?,
                cut.get(CUTLIST_ITEM_TIMES_DURATION).with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        CUTLIST_ITEM_TIMES_DURATION, i
                    )
                })?,
                CutlistItem::convert_time,
            )?
        } else {
            CutlistItem::new(
                cut.get(CUTLIST_ITEM_FRAMES_START).with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        CUTLIST_ITEM_FRAMES_START, i
                    )
                })?,
                cut.get(CUTLIST_ITEM_FRAMES_DURATION).with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        CUTLIST_ITEM_FRAMES_DURATION, i
                    )
                })?,
                CutlistItem::convert_frame,
            )?
        } {
            items.push(item);
        }
    }

    Ok(items)
}

/// cuts video file stored in in_path with mkvmerge using the cutlist
/// information in header and items and stores the cut video in out_path
fn cut_with_mkvmerge(
    in_path: &Path,
    out_path: &Path,
    header: &CutlistHeader,
    items: &[CutlistItem],
) -> anyhow::Result<()> {
    // assemble split parameter for mkvmerge
    let mut split_str = "".to_string();
    if header.with_frames {
        split_str += "parts-frames:"
    } else {
        split_str += "parts:"
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
