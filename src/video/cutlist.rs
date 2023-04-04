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

/// URI's for the retrieval of cut list data from the provider cutlist.at
const CUTLIST_RETRIEVE_HEADERS_URI: &str = "http://cutlist.at/getxml.php?name=";
const CUTLIST_RETRIEVE_LIST_DETAILS_URI: &str = "http://cutlist.at/getfile.php?id=";

/// Names for sections and attributes for the INI file of cutlist.at
const CUTLIST_ITEM_GENERAL_SECTION: &str = "General";
const CUTLIST_ITEM_NUM_OF_CUTS: &str = "NoOfCuts";
const CUTLIST_ITEM_CUT_SECTION: &str = "Cut";
const CUTLIST_ITEM_TIME_START: &str = "Start";
const CUTLIST_ITEM_TIME_DURATION: &str = "Duration";
const CUTLIST_ITEM_FRAMES_START: &str = "StartFrame";
const CUTLIST_ITEM_FRAMES_DURATION: &str = "DurationFrames";

/// Type of a cut list - i.e., whether the cut intervals are based on frame
/// numbers or time
#[derive(Clone)]
pub enum Kind {
    Frames,
    Time,
}
impl TryFrom<&str> for Kind {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "frames" => Ok(Kind::Frames),
            "time" => Ok(Kind::Time),
            _ => Err(anyhow!("'{}' is not a valid kind of a cut list", s)),
        }
    }
}

/// Header data to retrieve cut lists from a provider
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

/// Retrieves the headers of cut lists for a video from a provider. If no cut
/// list exists, an empty array but no error is returned.
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
                "Did not get a response for cut list header request for {}",
                file_name
            )
        })?
        .text()
        .with_context(|| format!("Could not parse cut list header response for {}", file_name))?;

    if response.is_empty() {
        return Err(anyhow!("Did not find a cut list for {:?}", file_name));
    }

    let mut headers: Vec<ProviderHeader> = vec![];

    let raw_headers: RawHeaders = quick_xml::de::from_str(&response)
        .with_context(|| format!("Could not parse cut list headers for {:?}", file_name))?;

    for raw_header in raw_headers.headers {
        // Do not accept cut lists with errors
        let num_errs = raw_header.errors.parse::<i32>();
        if num_errs.is_err() || num_errs.unwrap() > 0 {
            continue;
        }

        // Create default cutlist header
        let mut header = ProviderHeader {
            id: raw_header.id,
            rating: 0.0,
            kind: Kind::Frames,
        };

        // Parse rating
        if let Ok(rating) = raw_header.rating.parse::<f64>() {
            header.rating = rating;
        }

        // Parse frames indicator
        if let Ok(with_frames) = raw_header.with_frames.parse::<i32>() {
            header.kind = if with_frames == 1 {
                Kind::Frames
            } else {
                Kind::Time
            };
        }

        headers.push(header);
    }

    headers.sort();
    Ok(headers)
}

/// Cut/interval of a cutlist - i.e., a start and an end point
/// #[derive(Debug)]
pub struct Item {
    pub start: f64,
    pub end: f64,
}
impl Item {
    // Create a new cut ist item from a start point and a duration
    fn new(start: &str, duration: &str) -> anyhow::Result<Option<Item>> {
        // convert start and duration to floating point
        let start_f = start
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", start))?;
        let duration_f = duration
            .parse::<f64>()
            .with_context(|| format!("Could not parse {:?} to floating point", duration))?;

        // Cut list items with zero duration (i.e., start and end point are
        // equal) make no sense
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

/// Attribute name for start of a cut interval depending on the kind of the cut
/// list
fn item_attr_start(kind: &Kind) -> String {
    match kind {
        Kind::Frames => CUTLIST_ITEM_FRAMES_START.to_string(),
        _ => CUTLIST_ITEM_TIME_START.to_string(),
    }
}

/// Attribute name for the duration of a cut interval depending on the kind of
// the cut list
fn item_attr_duration(kind: &Kind) -> String {
    match kind {
        Kind::Frames => CUTLIST_ITEM_FRAMES_DURATION.to_string(),
        _ => CUTLIST_ITEM_TIME_DURATION.to_string(),
    }
}

lazy_static! {
    /// Reg exp for the internals string that can be specified on command line
    static ref RE_INTERVALS: Regex = Regex::new(r#"^(frames|time):((\[[^,]+,[^,]+\])+)$"#).unwrap();
}

/// Cut list, consisting of a kind/type and a list of cut intervals (items)
pub struct CutList {
    kind: Kind,
    pub items: Vec<Item>,
}

impl FromStr for CutList {
    type Err = anyhow::Error;

    /// Creates a cut list from an intervals string that was specified on command
    /// line
    fn from_str(intervals: &str) -> Result<Self, Self::Err> {
        if !RE_INTERVALS.is_match(intervals) {
            return Err(anyhow!("'{}' is not a valid intervals string", intervals));
        }

        let err_msg = format!("Cannot create cut list from '{}'", intervals);

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
                start: cut_str_to_f64(&list.kind, start_str).with_context(|| err_msg.clone())?,
                end: cut_str_to_f64(&list.kind, end_str).with_context(|| err_msg.clone())?,
            })
        }

        Ok(list)
    }
}

impl TryFrom<&ProviderHeader> for CutList {
    type Error = anyhow::Error;

    /// Retrieve a cut list from a cutlist provider using the given header
    fn try_from(header: &ProviderHeader) -> Result<Self, Self::Error> {
        let mut list = CutList {
            kind: header.kind.clone(),
            items: vec![],
        };

        // Retrieve cut list in INI format
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

        // Get number of cuts
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

        // Get cuts and create cut list items from them
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
    /// CHecks if a cut list is valid - i.e., whether the intervals do not
    /// overlap and whether the interval start is before its end
    pub fn is_valid(&self) -> bool {
        let last_item: Option<&Item> = None;

        for item in &self.items {
            if item.start > item.end {
                return false;
            }
            if let Some(last_item) = last_item {
                if last_item.end > item.start {
                    return false;
                }
            }
        }

        true
    }

    /// Creates the slit string that mkvmerge expects from a cut list
    pub fn to_mkvmerge_split_str(&self) -> String {
        let mut split_str = match self.kind {
            Kind::Frames => "parts-frames:",
            Kind::Time => "parts:",
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
    /// Reg exp for the interval start/end of a cut list of kind "time"
    static ref RE_TIME: Regex =
        Regex::new(r#"^(\d+):([0-5]\d)+:([0-5]\d)+(\.(\d{0,6}))*$"#).unwrap();
}

/// Converts the string representation of an interval start or end of a cut
/// interval into a floating point number
fn cut_str_to_f64(kind: &Kind, cut_str: &str) -> anyhow::Result<f64> {
    match kind {
        Kind::Frames => cut_str.parse::<f64>().with_context(|| {
            format!(
                "Could not parse frames cut string {:?} to floating point",
                cut_str
            )
        }),
        Kind::Time => {
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

/// Converts the floating point representation of an interval start or end of a
/// cut interval into a string
fn f64_to_cut_str(kind: &Kind, point: f64) -> String {
    let mut cut_str = "".to_string();

    match kind {
        Kind::Frames => write!(cut_str, "{:.0}", point)
            .expect("Cannot convert a point of a cut list of type frames to mkvmerge to string"),
        Kind::Time => {
            let time: u64 = (point * 1000000_f64) as u64;
            let (secs, subs) = (time / 1000000, time % 1000000);
            let (hours, rest) = (secs / 3600, secs % 3600);
            let (mins, rest) = (rest / 60, rest % 60);
            write!(cut_str, "{:02}:{:02}:{:02}.{:06}", hours, mins, rest, subs)
                .expect("Cannot convert a point of a cut list of type time to mkvmerge to string");
        }
    };

    cut_str
}
