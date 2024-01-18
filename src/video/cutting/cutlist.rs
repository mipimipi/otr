use anyhow::{anyhow, Context};
use ini::Ini;
use lazy_static::lazy_static;
use log::*;
use regex::Regex;
use serde::Deserialize;
use std::{
    cmp::{self, Eq, PartialEq},
    collections::HashMap,
    convert::TryFrom,
    fmt::{self, Debug, Display, Write},
    fs,
    hash::Hash,
    path::Path,
    str::{self, FromStr},
};

/// URI's for the retrieval of cut list data from the provider cutlist.at
const CUTLIST_RETRIEVE_HEADERS_URI: &str = "http://cutlist.at/getxml.php?name=";
const CUTLIST_RETRIEVE_LIST_DETAILS_URI: &str = "http://cutlist.at/getfile.php?id=";

/// cutlist.at (error) messages
const CUTLIST_AT_ERROR_ID_NOT_FOUND: &str = "Not found.";

/// Names for sections and attributes for the INI file of cutlist.at
const CUTLIST_ITEM_GENERAL_SECTION: &str = "General";
const CUTLIST_ITEM_NUM_OF_CUTS: &str = "NoOfCuts";
const CUTLIST_ITEM_META_SECTION: &str = "Meta";
const CUTLIST_ITEM_CUTLIST_ID: &str = "CutlistId";
const CUTLIST_ITEM_CUT_SECTION: &str = "Cut";
const CUTLIST_ITEM_TIME_START: &str = "Start";
const CUTLIST_ITEM_TIME_DURATION: &str = "Duration";
const CUTLIST_ITEM_FRAMES_START: &str = "StartFrame";
const CUTLIST_ITEM_FRAMES_DURATION: &str = "DurationFrames";

/// Type of cut list intervals - i.e., whether they are based on frame numbers or
/// time
#[derive(Clone, Default, Eq, Hash, PartialEq)]
pub enum Kind {
    #[default]
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
impl Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Kind::Frames => "frames",
                Kind::Time => "time",
            }
        )
    }
}

/// Cut list access type
pub enum AccessType<'a> {
    Auto,            // retrieve cut lists from provider and select one automatically
    Direct(&'a str), // direct access to cut list (as string consisting of intervals)
    File(&'a Path),  // retrieve cut list from file
    ID(u64),         // retrieve cutlist from provider by ID
}

/// Header data to retrieve cut lists from a provider
#[derive(Default)]
pub struct ProviderHeader {
    id: u64,
    rating: f64,
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
        errors: String,
    }

    trace!("\"{}\": Request cut lists from provider", file_name);

    let response = reqwest::blocking::get(CUTLIST_RETRIEVE_HEADERS_URI.to_string() + file_name)
        .with_context(|| {
            format!(
                "Did not get a response for cut list header request for \"{}\"",
                file_name
            )
        })?
        .text()
        .with_context(|| {
            format!(
                "Could not parse cut list header response for \"{}\"",
                file_name
            )
        })?;

    if response.is_empty() {
        trace!("\"{}\": No cut lists retrieved from provider", file_name);
        return Err(anyhow!("Did not find a cut list for \"{:?}\"", file_name));
    }

    let mut headers: Vec<ProviderHeader> = vec![];

    let raw_headers: RawHeaders = quick_xml::de::from_str(&response)
        .with_context(|| format!("Could not parse cut list headers for {:?}", file_name))?;

    trace!(
        "\"{}\": {} cut lists retrieved from provider",
        file_name,
        raw_headers.headers.len()
    );

    for raw_header in raw_headers.headers {
        // Do not accept cut lists with errors
        let num_errs = raw_header.errors.parse::<i32>();
        if num_errs.is_err() || num_errs.unwrap() > 0 {
            trace!(
                "\"{}\": Cut list {} has errors: Ignored",
                file_name,
                raw_header.id
            );
            continue;
        }

        // Create default cutlist header
        let mut header = ProviderHeader {
            id: raw_header.id,
            ..Default::default()
        };

        // Parse rating
        if let Ok(rating) = raw_header.rating.parse::<f64>() {
            header.rating = rating;
        }

        headers.push(header);
    }

    headers.sort();
    Ok(headers)
}

/// Cut/interval of a cutlist - i.e., a start and an end point
pub struct Item {
    pub start: f64,
    pub end: f64,
}
impl Item {
    // Create a new cut list item from a start point and a duration. If the new
    // item would have a duration of zero, None is returned.
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

    // Create a cut list item from a cut/interval from an ini structure
    fn from_ini(cutlist_ini: &Ini, cut_no: i32, kind: &Kind) -> anyhow::Result<Option<Item>> {
        let cut = cutlist_ini
            .section(Some(format!("{}{}", CUTLIST_ITEM_CUT_SECTION, cut_no)))
            .with_context(|| format!("Could not find section for cut no {}", cut_no))?;
        Item::new(
            cut.get(item_attr_start(kind)).with_context(|| {
                format!(
                    "Could not find attribute '{}' for cut no {}",
                    item_attr_start(kind),
                    cut_no
                )
            })?,
            cut.get(item_attr_duration(kind)).with_context(|| {
                format!(
                    "Could not find attribute '{}' for cut no {}",
                    item_attr_duration(kind),
                    cut_no
                )
            })?,
        )
    }
}

/// Attribute name for start of a cut interval depending on the kind, i.e. frame
/// numbers or time
fn item_attr_start(kind: &Kind) -> String {
    match kind {
        Kind::Frames => CUTLIST_ITEM_FRAMES_START.to_string(),
        _ => CUTLIST_ITEM_TIME_START.to_string(),
    }
}

/// Attribute name for the duration of a cut interval depending on the kind, i.e. frame
/// numbers or time
fn item_attr_duration(kind: &Kind) -> String {
    match kind {
        Kind::Frames => CUTLIST_ITEM_FRAMES_DURATION.to_string(),
        _ => CUTLIST_ITEM_TIME_DURATION.to_string(),
    }
}

lazy_static! {
    /// Reg exp for the intervals string
    static ref RE_INTERVALS: Regex = Regex::new(r#"^(frames|time):((\[[^,]+,[^,]+\])+)$"#).unwrap();
}

/// Cut list, consisting of intervals of frame numbers and/or times. At least one
/// of both must be there
#[derive(Default)]
pub struct CutList {
    items: HashMap<Kind, Vec<Item>>,
}
impl FromStr for CutList {
    type Err = anyhow::Error;

    /// Creates a cut list from an intervals string, i.e. "frames:[...]" or
    /// "[time:[...]"
    fn from_str(intervals: &str) -> Result<Self, Self::Err> {
        if !RE_INTERVALS.is_match(intervals) {
            return Err(anyhow!("'{}' is not a valid intervals string", intervals));
        }

        let err_msg = format!("Cannot create cut list from '{}'", intervals);

        let caps_intervals = RE_INTERVALS
            .captures(intervals)
            .unwrap_or_else(|| panic!("Cannot split intervals string since it is not valid"));

        let kind = Kind::try_from(
            caps_intervals
                .get(1)
                .unwrap_or_else(|| panic!("Cannot extract intervals type from intervals string"))
                .as_str(),
        )
        .expect("Cannot create cutlist kind from intervals string");

        // An intervals string can either be based on frame numbers or time, but
        // not both
        let mut cutlist: CutList = Default::default();
        cutlist.items.insert(kind.clone(), vec![]);
        let items = cutlist.items.get_mut(&kind).unwrap();

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

            items.push(Item {
                start: cut_str_to_f64(&kind, start_str).with_context(|| err_msg.clone())?,
                end: cut_str_to_f64(&kind, end_str).with_context(|| err_msg.clone())?,
            })
        }

        Ok(cutlist)
    }
}

/// Create a cut list from an ini structure
impl TryFrom<&Ini> for CutList {
    type Error = anyhow::Error;

    fn try_from(cutlist_ini: &Ini) -> Result<Self, Self::Error> {
        // Get cut list id
        fn cutlist_id(cutlist: &Ini) -> anyhow::Result<&str> {
            cutlist
                .section(Some(CUTLIST_ITEM_META_SECTION))
                .with_context(|| {
                    format!(
                        "Could not find section '{}' in cutlist",
                        CUTLIST_ITEM_META_SECTION
                    )
                })?
                .get(CUTLIST_ITEM_CUTLIST_ID)
                .with_context(|| {
                    format!(
                        "Could not find attribute '{}' in cutlist",
                        CUTLIST_ITEM_CUTLIST_ID
                    )
                })
        }

        let id = match cutlist_id(cutlist_ini) {
            Ok(id) => Some(id),
            _ => None,
        };

        // Get number of cuts
        let num_cuts = cutlist_ini
            .section(Some(CUTLIST_ITEM_GENERAL_SECTION))
            .with_context(|| {
                format!(
                    "Could not find section '{}' in cutlist '{}'",
                    CUTLIST_ITEM_GENERAL_SECTION,
                    id.unwrap_or("unknown")
                )
            })?
            .get(CUTLIST_ITEM_NUM_OF_CUTS)
            .with_context(|| {
                format!(
                    "Could not find attribute '{}' in cutlist '{}'",
                    CUTLIST_ITEM_NUM_OF_CUTS,
                    id.unwrap_or("unknown")
                )
            })?
            .parse::<i32>()
            .with_context(|| {
                format!(
                    "Could not parse attribute '{}' in cutlist '{}'",
                    CUTLIST_ITEM_NUM_OF_CUTS,
                    id.unwrap_or("unknown")
                )
            })?;

        // Retrieve cuts from ini structure and create a cut list from them
        let mut cutlist: CutList = Default::default();
        for i in 0..num_cuts {
            cutlist
                .extend_from_ini_cut(cutlist_ini, i)
                .with_context(|| {
                    format!(
                        "Could not read cuts of cut list '{}'",
                        id.unwrap_or("unknown")
                    )
                })?;
        }

        Ok(cutlist)
    }
}

/// Retrieve a cut list from a file
impl TryFrom<&Path> for CutList {
    type Error = anyhow::Error;

    fn try_from(cutlist_file: &Path) -> Result<Self, Self::Error> {
        CutList::try_from(
            &Ini::load_from_str(&fs::read_to_string(cutlist_file).with_context(|| {
                format!(
                    "Could not read from cut list file '{}'",
                    cutlist_file.display()
                )
            })?)
            .with_context(|| {
                format!(
                    "Could not parse response for cutlist '{}' as INI",
                    cutlist_file.display()
                )
            })?,
        )
    }
}

/// Retrieve a cut list from a cutlist provider by cutlist id
impl TryFrom<u64> for CutList {
    type Error = anyhow::Error;

    fn try_from(id: u64) -> Result<Self, Self::Error> {
        // Retrieve cut list by ID
        let response =
            reqwest::blocking::get(CUTLIST_RETRIEVE_LIST_DETAILS_URI.to_string() + &id.to_string())
                .with_context(|| format!("Did not get a response for requesting cutlist {}", id))?
                .text()
                .with_context(|| format!("Could not parse response for cutlist {} as text", id))?;
        if response == CUTLIST_AT_ERROR_ID_NOT_FOUND {
            return Err(anyhow!(
                "Cut list with ID={} does not exist at provider",
                id
            ));
        }

        // Parse cut list
        let cutlist_ini = Ini::load_from_str(&response)
            .with_context(|| format!("Could not parse response for cutlist {} as INI", id))?;

        CutList::try_from(&cutlist_ini)
    }
}

impl CutList {
    pub fn kinds(&self) -> Vec<Kind> {
        let mut kinds: Vec<Kind> = vec![];
        for kind in self.items.keys() {
            kinds.push(kind.clone());
        }
        kinds
    }

    /// Creates the split string that mkvmerge requires to cut a video
    pub fn to_mkvmerge_split_str(&self, kind: &Kind) -> anyhow::Result<String> {
        if !self.items.contains_key(kind) {
            return Err(anyhow!(
                "Cannot create mkvmerge split string: Cut list does not contain {} intervals",
                kind
            ));
        }

        let mut split_str = match kind {
            Kind::Frames => "parts-frames:",
            Kind::Time => "parts:",
        }
        .to_string();

        for (i, item) in self.items.get(kind).unwrap().iter().enumerate() {
            if i > 0 {
                split_str += ",+"
            }
            write!(
                split_str,
                "{}-{}",
                f64_to_cut_str(kind, item.start),
                f64_to_cut_str(kind, item.end)
            )
            .expect("Cannot convert cut list item to mkvmerge split string");
        }

        Ok(split_str)
    }

    /// Checks if a cut list is valid - i.e., whether at least one intervals
    /// array exists, whether the intervals of an array do not overlap and
    /// whether the start is before the end of each interval
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.items.is_empty() {
            return Err(anyhow!("Cut list does not contain intervals"));
        }

        // kind is just needed for the error messages
        fn validate_intervals(kind: &Kind, items: &Vec<Item>) -> anyhow::Result<()> {
            let last_item: Option<&Item> = None;

            for item in items {
                if item.start > item.end {
                    return Err(anyhow!(
                        "Cut list {} intervals are invalid: Start is after end",
                        kind
                    ));
                }
                if let Some(last_item) = last_item {
                    if last_item.end > item.start {
                        return Err(anyhow!("Cut list {} intervals overlap", kind));
                    }
                }
            }
            Ok(())
        }

        if self.items.contains_key(&Kind::Frames) {
            validate_intervals(&Kind::Frames, self.items.get(&Kind::Frames).unwrap())?;
        }
        if self.items.contains_key(&Kind::Time) {
            validate_intervals(&Kind::Time, self.items.get(&Kind::Time).unwrap())?;
        }

        Ok(())
    }

    // Retrieves cut number cut_no from ini structure, creates a cut list item
    // from it and appends it to the cut list
    fn extend_from_ini_cut(&mut self, cutlist_ini: &Ini, cut_no: i32) -> anyhow::Result<()> {
        if cut_no == 0 {
            // In case of the first cut, it is checked which kinds (frame numbers
            // and/or time) are supported. The corresponding item arrays are
            // created respectively
            for kind in [&Kind::Frames, &Kind::Time] {
                if let Ok(Some(item)) = Item::from_ini(cutlist_ini, cut_no, kind) {
                    self.items.insert(kind.clone(), vec![item]);
                }
            }
        } else {
            for kind in self.kinds() {
                if let Some(item) = Item::from_ini(cutlist_ini, cut_no, &kind).context(format!(
                    "Cut no {} does not contain {} information, though the cut list supprts that",
                    cut_no, kind,
                ))? {
                    self.items.get_mut(&kind).unwrap().push(item);
                }
            }
        }
        Ok(())
    }
}

lazy_static! {
    /// Reg exp for the interval start/end of a cut list of kind "time"
    static ref RE_TIME: Regex =
        Regex::new(r#"^(\d+):([0-5]\d)+:([0-5]\d)+(\.(\d{0,6}))*$"#).unwrap();
}

/// Converts the string representation of an interval start or end into a
/// floating point number
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
