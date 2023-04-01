use anyhow::{anyhow, Context};
use ini::Ini;
use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;
use std::{
    cmp,
    fmt::{Debug, Write},
    str::{self, FromStr},
};

/// URI's for the retrieval of cutlist data
const CUTLIST_RETRIEVE_HEADERS_URI: &str = "http://cutlist.at/getxml.php?name=";
const CUTLIST_RETRIEVE_LIST_DETAILS_URI: &str = "http://cutlist.at/getfile.php?id=";

/// Names for sections and attributs of INI file
const CUTLIST_ITEM_GENERAL_SECTION: &str = "General";
const CUTLIST_ITEM_NUM_OF_CUTS: &str = "NoOfCuts";
const CUTLIST_ITEM_CUT_SECTION: &str = "Cut";
const CUTLIST_ITEM_TIMES_START: &str = "Start";
const CUTLIST_ITEM_TIMES_DURATION: &str = "Duration";
const CUTLIST_ITEM_FRAMES_START: &str = "StartFrame";
const CUTLIST_ITEM_FRAMES_DURATION: &str = "DurationFrames";

/// Kind of a cut - i.e., whether it is expressed in frame numbers or times
#[derive(Clone)]
pub enum Kind {
    Frames,
    Times,
}
impl TryFrom<&str> for Kind {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "frames" => Ok(Kind::Frames),
            "times" => Ok(Kind::Times),
            _ => Err(anyhow!("'{}' is not a valid kind of a cut list", s)),
        }
    }
}

/// Header of a cutlist
pub struct ProviderHeader {
    id: u64,
    rating: f64,
    kind: Kind,
}
impl Eq for ProviderHeader {}
impl Ord for ProviderHeader {
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
impl PartialEq for ProviderHeader {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl PartialOrd for ProviderHeader {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl ProviderHeader {
    pub fn id(&self) -> u64 {
        self.id
    }
}

/// Retrieves the headers of potentially existing cutlists for a video. If no
/// cutlist exists, an empty array but no error is returned.
pub fn headers_from_provider(file_name: &str) -> anyhow::Result<Vec<ProviderHeader>> {
    #[derive(Debug, Deserialize)]
    struct RawHeaders {
        #[serde(rename = "cutlist")]
        headers: Vec<RawHeader>,
    }
    #[derive(Debug, Deserialize)]
    struct RawHeader {
        id: u64,
        rating: String,
        #[serde(rename = "withframes")]
        with_frames: String,
        errors: String,
    }

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
        return Err(anyhow!("Did not find cutlist for {:?}", file_name));
    }

    let mut headers: Vec<ProviderHeader> = vec![];

    let raw_headers: RawHeaders = quick_xml::de::from_str(&response)
        .with_context(|| format!("Could not parse cutlist headers for {:?}", file_name))?;

    for raw_header in raw_headers.headers {
        // don't accept cutlists with errors
        let num_errs = raw_header.errors.parse::<i32>();
        if num_errs.is_err() || num_errs.unwrap() > 0 {
            continue;
        }

        // create default cutlist header
        let mut header = ProviderHeader {
            id: raw_header.id,
            rating: 0.0,
            kind: Kind::Frames,
        };

        // parse rating
        if let Ok(rating) = raw_header.rating.parse::<f64>() {
            header.rating = rating;
        }

        // parse frames indicator
        if let Ok(with_frames) = raw_header.with_frames.parse::<i32>() {
            header.kind = if with_frames == 1 {
                Kind::Frames
            } else {
                Kind::Times
            };
        }

        headers.push(header);
    }

    headers.sort();
    Ok(headers)
}

/// Cut of a cutlist - i.e., a start and an end point
/// #[derive(Debug)]
pub struct Item {
    pub start: f64,
    pub end: f64,
}
impl Item {
    // Create a new CutListItem from a start point, a duration and the kind of
    // the cut
    fn new(start: &str, duration: &str) -> anyhow::Result<Option<Item>> {
        // convert start and duration to floating point
        let start_f = start
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", start))?;
        let duration_f = duration
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", duration))?;

        // cutlist item with zero duration (i.e., equal start and end make no sense)
        if duration_f > 0.0 {
            Ok(Some(Item {
                start: start_f,
                end: start_f + duration_f,
            }))
        } else {
            Ok(None)
        }
    }
}

/// Attribute name for start of a cut depending on the kind of the cut.
fn item_attr_start(kind: &Kind) -> String {
    match kind {
        Kind::Frames => CUTLIST_ITEM_FRAMES_START.to_string(),
        _ => CUTLIST_ITEM_TIMES_START.to_string(),
    }
}

/// Attribute name for the duration of a cut depending the kind of the cut.
fn item_attr_duration(kind: &Kind) -> String {
    match kind {
        Kind::Frames => CUTLIST_ITEM_FRAMES_DURATION.to_string(),
        _ => CUTLIST_ITEM_TIMES_DURATION.to_string(),
    }
}

lazy_static! {
    static ref RE_INTERVALS: Regex =
        Regex::new(r#"^(frames|times):((\[[^,]+,[^,]+\])+)$"#).unwrap();
}

pub struct CutList {
    kind: Kind,
    pub items: Vec<Item>,
}

impl FromStr for CutList {
    type Err = anyhow::Error;

    fn from_str(intervals: &str) -> Result<Self, Self::Err> {
        if !RE_INTERVALS.is_match(intervals) {
            return Err(anyhow!("'{}' is not a valid intervals string", intervals));
        }

        let caps_intervals = RE_INTERVALS
            .captures(intervals)
            .unwrap_or_else(|| panic!("Cannot split intervals string since it is not valid"));

        let mut list = CutList {
            kind: Kind::try_from(
                caps_intervals
                    .get(1)
                    .unwrap_or_else(|| {
                        panic!("Cannot extract intervals type from intervals string")
                    })
                    .as_str(),
            )
            .expect("Cannot create cutlist kind from intervals string"),
            items: vec![],
        };

        // Parse the actual intervals
        for interval in caps_intervals
            .get(2)
            .unwrap_or_else(|| panic!("Cannot extract intervals from intervals string"))
            .as_str()
            .split(']')
        {
            if interval.is_empty() {
                continue;
            }
            let mut points = interval[1..].split(',');
            let start_str = points
                .next()
                .unwrap_or_else(|| panic!("Cannot extract left boundary of interval"))
                .trim();
            let end_str = points
                .next()
                .unwrap_or_else(|| panic!("Cannot extract right boundary of interval"))
                .trim();

            list.items.push(Item {
                start: cut_str_to_f64(&list.kind, start_str)?,
                end: cut_str_to_f64(&list.kind, end_str)?,
            })
        }

        Ok(list)
    }
}

impl TryFrom<&ProviderHeader> for CutList {
    type Error = anyhow::Error;

    /// Retrieve the cutlist (i.e., the different cuts) for a cutlist provider
    // using the given header
    fn try_from(header: &ProviderHeader) -> Result<Self, Self::Error> {
        let mut list = CutList {
            kind: header.kind.clone(),
            items: vec![],
        };

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
        let raw_list = Ini::load_from_str(&response).with_context(|| {
            format!("Could not parse response for cutlist {} as INI", header.id)
        })?;

        // get number of cuts
        let num_cuts = raw_list
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
        for i in 0..num_cuts {
            let cut = raw_list
                .section(Some(format!("{}{}", CUTLIST_ITEM_CUT_SECTION, i)))
                .with_context(|| {
                    format!(
                        "Could not find section for cut no {} in cutlist {}",
                        i, header.id
                    )
                })?;
            if let Some(item) = Item::new(
                cut.get(item_attr_start(&header.kind)).with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        item_attr_start(&header.kind),
                        i
                    )
                })?,
                cut.get(item_attr_duration(&header.kind)).with_context(|| {
                    format!(
                        "Could not find attribute '{}' for cut no {}",
                        item_attr_duration(&header.kind),
                        i
                    )
                })?,
            )? {
                list.items.push(item);
            }
        }

        Ok(list)
    }
}

impl CutList {
    pub fn to_mkvmerge_split_str(&self) -> String {
        let mut split_str = match self.kind {
            Kind::Frames => "parts-frames:",
            Kind::Times => "parts:",
        }
        .to_string();

        for (i, item) in self.items.iter().enumerate() {
            if i > 0 {
                split_str += ",+"
            }
            write!(
                split_str,
                "{}-{}",
                f64_to_cut_str(&self.kind, item.start),
                f64_to_cut_str(&self.kind, item.end)
            )
            .expect("Cannot convert cut list item to mkvmerge split string");
        }

        split_str
    }
}

lazy_static! {
    static ref RE_TIME: Regex =
        Regex::new(r#"^(\d+):([0-5]\d)+:([0-5]\d)+(\.(\d{0,6}))*$"#).unwrap();
}

fn cut_str_to_f64(kind: &Kind, cut_str: &str) -> anyhow::Result<f64> {
    match kind {
        Kind::Frames => cut_str.parse::<f64>().with_context(|| {
            format!(
                "Could not parse frames cut string {:?} to floating point",
                cut_str
            )
        }),
        Kind::Times => {
            if !RE_TIME.is_match(cut_str) {
                return Err(anyhow!("'{}' is not a valid time cut string", cut_str));
            }

            let caps = RE_TIME
                .captures(cut_str)
                .unwrap_or_else(|| panic!("Time cut string has invalid format"));
            let hours = caps
                .get(1)
                .unwrap_or_else(|| panic!("Cannot extract hours from time cut string"))
                .as_str()
                .parse::<f64>()
                .expect("Cannot convert hours from time cut string to float");
            let mins = caps
                .get(2)
                .unwrap_or_else(|| panic!("Cannot extract minutes from time cut string"))
                .as_str()
                .parse::<f64>()
                .expect("Cannot convert minutes from time cut string to float");
            let secs = caps
                .get(3)
                .unwrap_or_else(|| panic!("Cannot extract seconds from time cut string"))
                .as_str()
                .parse::<f64>()
                .expect("Cannot convert seconds from time cut string to float");

            Ok(hours * 3600.0
                + mins * 60.0
                + secs
                + match caps.get(5) {
                    Some(subs_match) => {
                        let subs_str = subs_match.as_str();
                        let subs = subs_str
                            .parse::<f64>()
                            .expect("Cannot convert sub seconds from time cut string to float");
                        subs * f64::powf(10_f64, -(subs_str.len() as f64))
                    }
                    None => 0.0,
                })
        }
    }
}

fn f64_to_cut_str(kind: &Kind, point: f64) -> String {
    let mut cut_str = "".to_string();

    match kind {
        Kind::Frames => write!(cut_str, "{:.0}", point)
            .expect("Cannot convert a point of a cut list of type frames to mkvmerge to string"),
        Kind::Times => {
            let time: u64 = (point * 1000000_f64) as u64;
            let (secs, subs) = (time / 1000000, time % 1000000);
            let (hours, rest) = (secs / 3600, secs % 3600);
            let (mins, rest) = (rest / 60, rest % 60);
            write!(cut_str, "{:02}:{:02}:{:02}.{:06}", hours, mins, rest, subs)
                .expect("Cannot convert a point of a cut list of type times to mkvmerge to string");
        }
    };

    cut_str
}
