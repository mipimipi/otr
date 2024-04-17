use anyhow::{anyhow, Context};
use ini::Ini;
use lazy_static::lazy_static;
use log::*;
use regex::Regex;
use reqwest::{
    blocking::{multipart, Client},
    StatusCode,
};
use serde::Deserialize;
use std::{
    cmp::{self, Eq, PartialEq},
    collections::HashMap,
    convert::TryFrom,
    env,
    fmt::{self, Debug, Display},
    fs,
    hash::Hash,
    path::Path,
    str,
};

/// URI's for retrieving and submitting cut list data from/to cutlist.at
const CUTLIST_RETRIEVE_HEADERS_URI: &str = "http://cutlist.at/getxml.php?name=";
const CUTLIST_RETRIEVE_LIST_DETAILS_URI: &str = "http://cutlist.at/getfile.php?id=";
const CUTLIST_SUBMIT_LIST_URI: &str = "http://cutlist.at";

/// cutlist.at (error) messages
const CUTLIST_AT_ERROR_ID_NOT_FOUND: &str = "Not found.";

/// Names for sections and attributes for the INI file of cutlist.at
const CUTLIST_GENERAL_SECTION: &str = "General";
const CUTLIST_INFO_SECTION: &str = "Info";
const CUTLIST_META_SECTION: &str = "Meta";
const CUTLIST_APPLICATION: &str = "Application";
const CUTLIST_VERSION: &str = "Version";
const CUTLIST_INTENDED_CUT_APP: &str = "IntendedCutApplicationName";
const CUTLIST_APPLY_TO_FILE: &str = "ApplyToFile";
const CUTLIST_ORIG_FILE_SIZE: &str = "OriginalFileSizeBytes";
const CUTLIST_NUM_OF_CUTS: &str = "NoOfCuts";
const CUTLIST_ID: &str = "CutlistId";
const CUTLIST_CUT_SECTION: &str = "Cut";
const CUTLIST_RATING_BY_AUTHOR: &str = "RatingByAuthor";
const CUTLIST_ITEM_TIME_START: &str = "Start";
const CUTLIST_ITEM_TIME_DURATION: &str = "Duration";
const CUTLIST_ITEM_FRAMES_START: &str = "StartFrame";
const CUTLIST_ITEM_FRAMES_DURATION: &str = "DurationFrames";

// Regular expressions
lazy_static! {
    // Parse cut list ID from cutlist.at's response to the submission request
    static ref RE_CUTLIST_ID: Regex =
        Regex::new(r"^ID=(\d+).*").unwrap();
    /// Reg exp for the intervals string
    static ref RE_INTERVALS: Regex = Regex::new(r#"^(frames|time):((\[[^,]+,[^,]+\])+)$"#).unwrap();
    /// Reg exp for the interval start/end of a cut list of kind "time"
    static ref RE_TIME: Regex =
        Regex::new(r#"^(\d+):([0-5]\d)+:([0-5]\d)+(\.(\d{0,6}))*$"#).unwrap();
}

/// Display an option: Print value as string in case of it is Some(value),
/// "unknown" otherwise
macro_rules! display_option {
    ($id:expr) => {
        if let Some(_id) = $id {
            format!("{}", _id)
        } else {
            "unknown".to_string()
        }
    };
}

/// Alias for cut list rating
pub type Rating = u8;

/// Alias for cut list ID
pub type ID = u64;

/// Fields that control processing of cut lists. Structure is made for the API
/// of this crate
pub struct Ctrl<'a> {
    pub submit: bool,
    pub rating: Rating,
    pub min_rating: Option<Rating>,
    pub access_token: Option<&'a str>,
    pub access_type: AccessType<'a>,
}

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
            _ => Err(anyhow!("\"{}\" is not a valid kind of a cut list", s)),
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
    ID(ID),          // retrieve cut list from provider by ID
}

/// Header data to retrieve cut lists from a provider
#[derive(Default)]
pub struct ProviderHeader {
    id: ID,
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
    pub fn id(&self) -> ID {
        self.id
    }
}

/// Retrieves the headers of cut lists for a video from a provider. If no cut
/// list exists, an empty array but no error is returned.
/// file_name is the name of the video file. min_rating specifies the minimum
/// rating a cut list must have to be accepted
pub fn headers_from_provider(
    file_name: &str,
    min_rating: Option<Rating>,
) -> anyhow::Result<Vec<ProviderHeader>> {
    #[derive(Debug, Deserialize)]
    struct RawHeaders {
        #[serde(rename = "cutlist")]
        headers: Vec<RawHeader>,
    }
    #[derive(Debug, Deserialize)]
    struct RawHeader {
        id: ID,
        rating: String,
        #[serde(rename = "ratingbyauthor")]
        rating_by_author: String,
        errors: String,
    }

    trace!("\"{}\": Request cut lists from provider", file_name);

    let response = reqwest::blocking::get(CUTLIST_RETRIEVE_HEADERS_URI.to_string() + file_name)
        .with_context(|| "Did not get a response for cut list header request")?
        .text()
        .with_context(|| "Could not parse cut list header response")?;

    if response.is_empty() {
        trace!("\"{}\": No cut lists retrieved from provider", file_name);
        return Err(anyhow!("No cut list could be retrieved"));
    }

    let mut headers: Vec<ProviderHeader> = vec![];

    let raw_headers: RawHeaders =
        quick_xml::de::from_str(&response).with_context(|| "Could not parse cut list headers")?;

    trace!(
        "\"{}\": {} cut lists retrieved from provider",
        file_name,
        raw_headers.headers.len()
    );

    for raw_header in raw_headers.headers {
        // Do not accept cut lists with errors
        let num_errs = raw_header.errors.parse::<i32>();
        if num_errs.is_err() || num_errs.unwrap() > 0 {
            warn!(
                "\"{}\": Cut list {} has errors: Ignored",
                file_name, raw_header.id
            );
            continue;
        }

        // Create default cut list header
        let mut header = ProviderHeader {
            id: raw_header.id,
            ..Default::default()
        };

        // Parse rating. First try general rating. If that does not exist, try
        // the rating by the author of the cut list
        if let Ok(_rating) = raw_header.rating.parse::<f64>() {
            header.rating = _rating;
        } else if let Ok(_rating) = raw_header.rating_by_author.parse::<f64>() {
            header.rating = _rating;
        }

        // Check if rating is good enough
        if let Some(_rating) = min_rating {
            if header.rating < _rating as f64 {
                info!(
                    "Rating of cut list {} for {} is too low",
                    header.id, file_name
                );
            } else {
                headers.push(header);
            }
        } else {
            headers.push(header);
        }
    }

    headers.sort();
    Ok(headers)
}

/// Cut/interval of a cut list - i.e., a start and an end point
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
            .with_context(|| format!("Could not parse \"{}\" to floating point", start))?;
        let duration_f = duration
            .parse::<f64>()
            .with_context(|| format!("Could not parse \"{}\" to floating point", duration))?;

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
            .section(Some(format!("{}{}", CUTLIST_CUT_SECTION, cut_no)))
            .with_context(|| format!("Could not find section for cut no {}", cut_no))?;
        Item::new(
            cut.get(item_attr_start(kind)).with_context(|| {
                format!(
                    "Could not find attribute \"{}\" for cut no {}",
                    item_attr_start(kind),
                    cut_no
                )
            })?,
            cut.get(item_attr_duration(kind)).with_context(|| {
                format!(
                    "Could not find attribute \"{}\" for cut no {}",
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

/// Cut list, consisting of intervals of frame numbers and/or times. At least one
/// of both must be there
#[derive(Default)]
pub struct Cutlist {
    id: Option<ID>,
    items: HashMap<Kind, Vec<Item>>, // intervals/cuts of cut list
}

/// Create a cut list from an ini structure
impl TryFrom<&Ini> for Cutlist {
    type Error = anyhow::Error;

    fn try_from(cutlist_ini: &Ini) -> Result<Self, Self::Error> {
        let mut cutlist = Cutlist {
            // Get cut list id. This is done in a separate step early in the
            // parsing process of the entire INI structure to be able to use the
            // id in error messages
            id: match cutlist_ini
                .section(Some(CUTLIST_META_SECTION))
                .with_context(|| {
                    format!(
                        "Could not find section \"{}\" in cut list",
                        CUTLIST_META_SECTION
                    )
                })?
                .get(CUTLIST_ID)
                .with_context(|| format!("Could not find attribute \"{}\" in cut list", CUTLIST_ID))
            {
                Ok(_id) => {
                    Some(str::parse(_id).context(
                        "Cut list ID does not have the correct format (must be a number)",
                    )?)
                }
                _ => None,
            },
            ..Default::default()
        };

        // Get number of cuts
        let num_cuts = cutlist_ini
            .section(Some(CUTLIST_GENERAL_SECTION))
            .with_context(|| {
                format!(
                    "Could not find section \"{}\" in cutlist ID={}",
                    CUTLIST_GENERAL_SECTION,
                    display_option!(cutlist.id)
                )
            })?
            .get(CUTLIST_NUM_OF_CUTS)
            .with_context(|| {
                format!(
                    "Could not find attribute \"{}\" in cutlist ID={}",
                    CUTLIST_NUM_OF_CUTS,
                    display_option!(cutlist.id)
                )
            })?
            .parse::<i32>()
            .with_context(|| {
                format!(
                    "Could not parse attribute \"{}\" in cutlist ID={}",
                    CUTLIST_NUM_OF_CUTS,
                    display_option!(cutlist.id)
                )
            })?;

        // Retrieve cuts from ini structure and create a cut list from them
        for i in 0..num_cuts {
            cutlist
                .extend_from_ini_cut(cutlist_ini, i)
                .with_context(|| {
                    format!(
                        "Could not read cuts of cut list ID={}",
                        display_option!(cutlist.id)
                    )
                })?;
        }

        cutlist
            .validate()
            .context("INI data does not represent a valid cut list")?;

        Ok(cutlist)
    }
}

/// Retrieve a cut list from a file
impl TryFrom<&Path> for Cutlist {
    type Error = anyhow::Error;

    fn try_from(cutlist_file: &Path) -> Result<Self, Self::Error> {
        Cutlist::try_from(
            &Ini::load_from_str(&fs::read_to_string(cutlist_file).with_context(|| {
                format!(
                    "Could not read from cut list file \"{}\"",
                    cutlist_file.display()
                )
            })?)
            .with_context(|| {
                format!(
                    "Could not parse response for cut list \"{}\" as INI",
                    cutlist_file.display()
                )
            })?,
        )
        .context(format!(
            "\"{}\" does not contain a valid cut list",
            cutlist_file.display()
        ))
    }
}

/// Retrieve a cut list from a cut list provider by cut list id
impl TryFrom<ID> for Cutlist {
    type Error = anyhow::Error;

    fn try_from(id: ID) -> Result<Self, Self::Error> {
        // Retrieve cut list by ID
        let response =
            reqwest::blocking::get(CUTLIST_RETRIEVE_LIST_DETAILS_URI.to_string() + &id.to_string())
                .with_context(|| format!("Did not get a response for requesting cut list {}", id))?
                .text()
                .with_context(|| format!("Could not parse response for cut list {} as text", id))?;
        if response == CUTLIST_AT_ERROR_ID_NOT_FOUND {
            return Err(anyhow!(
                "Cut list with ID={} does not exist at provider",
                id
            ));
        }

        // Parse cut list
        let cutlist_ini = Ini::load_from_str(&response)
            .with_context(|| format!("Could not parse response for cut list {} as INI", id))?;

        Cutlist::try_from(&cutlist_ini)
    }
}

impl Cutlist {
    /// Creates a cut list from an intervals string, i.e. "frames:[...]" or
    /// "[time:[...]"
    pub fn try_from_intervals(intervals: &str) -> anyhow::Result<Cutlist> {
        if !RE_INTERVALS.is_match(intervals) {
            return Err(anyhow!("\"{}\" is not a valid intervals string", intervals));
        }

        let err_msg = format!("Cannot create cut list from \"{}\"", intervals);

        let caps_intervals = RE_INTERVALS
            .captures(intervals)
            .unwrap_or_else(|| panic!("Cannot split intervals string since it is not valid"));

        let kind = Kind::try_from(
            caps_intervals
                .get(1)
                .unwrap_or_else(|| panic!("Cannot extract intervals type from intervals string"))
                .as_str(),
        )
        .expect("Cannot create cut list kind from intervals string");

        // An intervals string can either be based on frame numbers or time, but
        // not both
        let mut cutlist = Cutlist::default();
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

        cutlist
            .validate()
            .context(format!("{} does not represent a valid cut list", intervals))?;

        Ok(cutlist)
    }

    /// Return an iterator for the cut list items of a ceratin kind
    pub fn items(&self, kind: &Kind) -> anyhow::Result<std::slice::Iter<'_, Item>> {
        match self.items.get(kind) {
            Some(items) => Ok(items.iter()),
            None => Err(anyhow!("Cut list is not of kind \"{}\"", kind)),
        }
    }

    /// Checks is cut list contain items of a ceratin kind
    pub fn is_of_kind(&self, kind: &Kind) -> bool {
        self.items.contains_key(kind)
    }

    /// Iterator over the kinds of a cut list
    pub fn kinds(&self) -> std::collections::hash_map::Keys<'_, Kind, std::vec::Vec<Item>> {
        self.items.keys()
    }

    /// Submit cut list to cutlist.at and set the cut list ID in self from the
    /// response
    pub fn submit<P, Q>(
        &mut self,
        video_path: P,
        tmp_dir: Q,
        access_token: &str,
        rating: Rating,
    ) -> anyhow::Result<()>
    where
        P: AsRef<Path>,
        Q: AsRef<Path>,
    {
        let file_name = video_path.as_ref().file_name().unwrap().to_str().unwrap();
        let cutlist_file = tmp_dir.as_ref().join(format!("{}.cutlist", file_name));

        // Generate INI structure and write it to a file
        self.to_ini(video_path.as_ref(), rating)?
            .write_to_file(cutlist_file.as_path())
            .context(format!(
                "Could not write cut list to file \"{}\"",
                cutlist_file.display()
            ))?;

        // Upload file to cutlist.at
        let response = Client::new()
            .post(format!("{}/{}/", CUTLIST_SUBMIT_LIST_URI, access_token))
            .multipart(
                multipart::Form::new()
                    .file("userfile[]", cutlist_file)
                    .context("Could not create cut list submission request")?,
            )
            .send()
            .with_context(|| "Did not get a response for cut list submission request")?;

        // Process response
        match response.status() {
            StatusCode::OK => {
                self.id =
                    Some(
                        str::parse(
                            RE_CUTLIST_ID
                                .captures(&response.text().with_context(|| {
                                    "Could not parse cut list submission response"
                                })?)
                                .unwrap()
                                .get(1)
                                .unwrap()
                                .as_str(),
                        )
                        .unwrap(),
                    );

                info!(
                    "Submitted cut list ID {} for \"{}\"",
                    self.id.unwrap(),
                    file_name,
                );

                Ok(())
            }
            _ => {
                let resp_txt = response
                    .text()
                    .with_context(|| "Could not parse cut list submission response")?;

                Err(anyhow!(if resp_txt.is_empty() {
                    "Received no response text for submission request".to_string()
                } else {
                    resp_txt
                }))
            }
        }
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
            for kind in [&Kind::Frames, &Kind::Time] {
                if self.is_of_kind(kind) {
                    if let Some(item) =
                        Item::from_ini(cutlist_ini, cut_no, kind).context(format!(
                    "Cut no {} does not contain {} information, though the cut list supports that",
                    cut_no, kind,
                ))? {
                        self.items.get_mut(kind).unwrap().push(item);
                    }
                }
            }
        }
        Ok(())
    }

    /// Length of cut list (i.e., the number of cuts). If the cut list has both,
    /// frames and time intervals/cuts, the number of cuts must be equal, since
    /// otherwise the cut list was invalid
    fn len(&self) -> usize {
        if self.is_of_kind(&Kind::Frames) {
            return self.items.get(&Kind::Frames).unwrap().len();
        }
        if self.is_of_kind(&Kind::Time) {
            return self.items.get(&Kind::Time).unwrap().len();
        }
        0
    }

    /// Create an INI structure from a cut list. video_path and rating are
    /// used to create the corresponding mandatory fields in the INI structure
    fn to_ini<P>(&self, video_path: P, rating: Rating) -> anyhow::Result<Ini>
    where
        P: AsRef<Path>,
    {
        let mut cutlist_ini = Ini::new();
        let num_cuts = self.len();
        let file_name = video_path.as_ref().file_name().unwrap().to_str().unwrap();

        // Section "[General]"
        cutlist_ini
            .with_section(Some(CUTLIST_GENERAL_SECTION))
            .set(CUTLIST_APPLICATION, env!("CARGO_PKG_NAME"))
            .set(CUTLIST_VERSION, env!("CARGO_PKG_VERSION"))
            .set(CUTLIST_INTENDED_CUT_APP, "mkvmerge")
            .set(CUTLIST_NUM_OF_CUTS, format!("{}", num_cuts))
            .set(CUTLIST_APPLY_TO_FILE, file_name)
            .set(
                CUTLIST_ORIG_FILE_SIZE,
                format!(
                    "{}",
                    fs::metadata(video_path.as_ref())
                        .context("Cannot create INI structure for cut list")?
                        .len()
                ),
            );

        // Sections "[CutN]" for N=0,...,<number-of-cuts>-1
        for i in 0..num_cuts {
            for kind in self.items.keys() {
                cutlist_ini
                    .with_section(Some(format!("{}{}", CUTLIST_CUT_SECTION, i)))
                    .set(
                        item_attr_start(kind),
                        format!("{}", self.items.get(kind).unwrap()[i].start),
                    )
                    .set(
                        item_attr_duration(kind),
                        format!(
                            "{}",
                            self.items.get(kind).unwrap()[i].end
                                - self.items.get(kind).unwrap()[i].start,
                        ),
                    );
            }
        }

        // Section "[Info]"
        cutlist_ini
            .with_section(Some(CUTLIST_INFO_SECTION))
            .set(CUTLIST_RATING_BY_AUTHOR, format!("{}", rating));

        Ok(cutlist_ini)
    }

    /// Checks if a cut list is valid - i.e., whether at least one intervals
    /// array exists, whether the intervals of an array do not overlap and
    /// whether the start is before the end of each interval
    fn validate(&self) -> anyhow::Result<()> {
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

        // If cut list contains time and frames intervals, both must have the
        // same number of items
        if self.is_of_kind(&Kind::Frames)
            && self.is_of_kind(&Kind::Time)
            && self.items.get(&Kind::Frames).unwrap().len()
                != self.items.get(&Kind::Time).unwrap().len()
        {
            return Err(anyhow!(
                "Cut list has time and frames intervals, but the number of cuts differ"
            ));
        }

        if self.is_of_kind(&Kind::Frames) {
            validate_intervals(&Kind::Frames, self.items.get(&Kind::Frames).unwrap())?;
        }
        if self.is_of_kind(&Kind::Time) {
            validate_intervals(&Kind::Time, self.items.get(&Kind::Time).unwrap())?;
        }

        Ok(())
    }
}

/// Converts the string representation of an interval start or end into a
/// floating point number
fn cut_str_to_f64(kind: &Kind, cut_str: &str) -> anyhow::Result<f64> {
    match kind {
        Kind::Frames => cut_str.parse::<f64>().with_context(|| {
            format!(
                "Could not parse frames cut string \"{}\" to floating point",
                cut_str
            )
        }),
        Kind::Time => {
            if !RE_TIME.is_match(cut_str) {
                return Err(anyhow!("\"{}\" is not a valid time cut string", cut_str));
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
