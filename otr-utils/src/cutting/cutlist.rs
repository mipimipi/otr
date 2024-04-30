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
    convert::TryFrom,
    env,
    fmt::Debug,
    fs,
    path::Path,
    str::{self, FromStr},
};

use super::interval::{self, Boundary, BoundaryType, Frame, Interval, Time};

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
    static ref RE_INTERVALS: Regex = Regex::new(r#"^(?<type>frames|times):(?<intervals>\[.+\])$"#).unwrap();
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

    for raw_header in &raw_headers.headers {
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

/// Create a cut interval from an INI structure. If an interval of length zero
/// would be created, Ok(None) is returned. Ok(Some(Interval<B>)) otherwise
fn interval_from_ini<B>(cutlist_ini: &Ini, cut_no: usize) -> anyhow::Result<Interval<B>>
where
    B: Boundary,
{
    let err_msg = format!(
        "Could not create interval from INI structure for cut internal no {}",
        cut_no
    );

    let cut_ini = cutlist_ini
        .section(Some(format!("{}{}", CUTLIST_CUT_SECTION, cut_no)))
        .context(format!("Could not find section for cut no {}", cut_no))
        .context(err_msg.clone())?;

    let (start, duration) = (
        cut_ini
            .get(item_attr_start(BoundaryType::from_str(
                std::any::type_name::<B>(),
            )?))
            .context({
                format!(
                    "Could not find attribute \"{}\" for cut no {}",
                    item_attr_start(BoundaryType::from_str(std::any::type_name::<B>()).unwrap()),
                    cut_no
                )
            })
            .ok(),
        cut_ini
            .get(item_attr_duration(BoundaryType::from_str(
                std::any::type_name::<B>(),
            )?))
            .context({
                format!(
                    "Could not find attribute \"{}\" for cut no {}",
                    item_attr_duration(BoundaryType::from_str(std::any::type_name::<B>()).unwrap()),
                    cut_no
                )
            })
            .ok(),
    );

    if start.is_none() {
        return Err(
            anyhow!("Could not retrieve start attribute from INI structure").context(err_msg),
        );
    }
    if duration.is_none() {
        return Err(
            anyhow!("Could not retrieve duration attribute from INI structure").context(err_msg),
        );
    }

    // Though start.unwrap() and duration.unwrap() are &str, a conversion to f64
    // is done since the strings are no valid time strings (in case the interval
    // is provided as time interval)
    Ok(Interval::<B>::from_start_duration(
        B::from(start.unwrap().parse::<f64>().context(err_msg.clone())?),
        B::from(duration.unwrap().parse::<f64>().context(err_msg.clone())?),
    ))
}

/// Attribute name for start of a cut interval depending on the boundary type -
/// i.e., frame or time
fn item_attr_start(btype: BoundaryType) -> String {
    if btype == BoundaryType::Frame {
        CUTLIST_ITEM_FRAMES_START.to_string()
    } else {
        CUTLIST_ITEM_TIME_START.to_string()
    }
}

/// Attribute name for the duration of a cut interval depending on the boundary
/// type - i.e., frame or time
fn item_attr_duration(btype: BoundaryType) -> String {
    if btype == BoundaryType::Frame {
        CUTLIST_ITEM_FRAMES_DURATION.to_string()
    } else {
        CUTLIST_ITEM_TIME_DURATION.to_string()
    }
}

/// Cut list, consisting of intervals of frame numbers and/or times. At least one
/// of both must be there
#[derive(Default)]
pub struct Cutlist {
    id: Option<ID>,
    frame_intervals: Option<Vec<Interval<Frame>>>,
    time_intervals: Option<Vec<Interval<Time>>>,
}

/// Create a cut list from an ini structure
impl TryFrom<&Ini> for Cutlist {
    type Error = anyhow::Error;

    /// Create a cut list from an INI structure
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
            .parse::<usize>()
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

    /// Create a cut list from a file
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
    /// "times:[...]"
    pub fn try_from_intervals(intervals: &str) -> anyhow::Result<Cutlist> {
        let err_msg = format!(
            "Could not create cut list from intervals string \"{}\"",
            intervals
        );

        if !RE_INTERVALS.is_match(intervals) {
            return Err(
                anyhow!("\"{}\" is not a valid intervals string", intervals).context(err_msg)
            );
        }

        let mut cutlist = Cutlist::default();

        // Extract boundary type and intervals string
        let btype = BoundaryType::from_str(
            RE_INTERVALS
                .captures(intervals)
                .unwrap()
                .name("type")
                .unwrap()
                .as_str(),
        )
        .context(err_msg.clone())?;
        let intervals = RE_INTERVALS
            .captures(intervals)
            .unwrap()
            .name("intervals")
            .unwrap()
            .as_str();

        // Create interval list from string
        if btype == BoundaryType::Frame {
            cutlist.frame_intervals =
                Some(interval::intervals_from_str::<Frame>(intervals).context(err_msg.clone())?)
        } else {
            cutlist.time_intervals =
                Some(interval::intervals_from_str::<Time>(intervals).context(err_msg.clone())?)
        }

        cutlist
            .validate()
            .context(format!("{} does not represent a valid cut list", intervals))
            .context(err_msg)?;

        Ok(cutlist)
    }

    /// Whether or not cut list has frame intervals
    pub fn has_frame_intervals(&self) -> bool {
        self.frame_intervals.is_some()
    }

    /// Whether or not cut list has time intervals
    pub fn has_time_intervals(&self) -> bool {
        self.time_intervals.is_some()
    }

    /// Provide an iterator for frame intervals
    pub fn frame_intervals(&self) -> anyhow::Result<std::slice::Iter<'_, Interval<Frame>>> {
        match &self.frame_intervals {
            Some(frame_intervals) => Ok(frame_intervals.iter()),
            None => Err(anyhow!("Cut list does not have frame intervals")),
        }
    }

    /// Provide an iterator for frame intervals
    pub fn time_intervals(&self) -> anyhow::Result<std::slice::Iter<'_, Interval<Time>>> {
        match &self.time_intervals {
            Some(time_intervals) => Ok(time_intervals.iter()),
            None => Err(anyhow!("Cut list does not have time intervals")),
        }
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

    /// Retrieves cut interval number cut_no from ini structure, creates a cut
    /// list item from it and appends it to the cut list
    fn extend_from_ini_cut(&mut self, cutlist_ini: &Ini, cut_no: usize) -> anyhow::Result<()> {
        let err_msg = format!(
            "Could not extend cut list by cut interval number {}",
            cut_no
        );

        // Try to retrieve and add frame interval
        let interval = interval_from_ini::<Frame>(cutlist_ini, cut_no).context(err_msg.clone())?;
        if !interval.is_empty() {
            if cut_no == 0 {
                self.frame_intervals = Some(vec![interval]);
            } else if self.has_frame_intervals() {
                self.frame_intervals.as_mut().unwrap().push(interval)
            } else {
                return Err(anyhow!(
                    "Cannot add frame interval to cut list since it had no frame intervals so far"
                )
                .context(err_msg));
            }
        }
        // Try to retrieve and add time interval
        let interval = interval_from_ini::<Time>(cutlist_ini, cut_no).context(err_msg.clone())?;
        if !interval.is_empty() {
            if cut_no == 0 {
                self.time_intervals = Some(vec![interval]);
            } else if self.has_time_intervals() {
                self.time_intervals.as_mut().unwrap().push(interval)
            } else {
                return Err(anyhow!(
                    "Cannot add time interval to cut list since it had no time intervals so far"
                )
                .context(err_msg));
            }
        }

        Ok(())
    }

    /// Length of cut list (i.e., the number of cuts). If the cut list has both,
    /// frame and time intervals, the number of cuts must be equal, since
    /// otherwise the cut list is invalid
    fn len(&self) -> usize {
        if self.has_frame_intervals() {
            return self.frame_intervals.as_ref().unwrap().len();
        }
        if self.has_time_intervals() {
            return self.time_intervals.as_ref().unwrap().len();
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
        let file_name = video_path.as_ref().file_name().unwrap().to_str().unwrap();

        // Section "[General]"
        cutlist_ini
            .with_section(Some(CUTLIST_GENERAL_SECTION))
            .set(CUTLIST_APPLICATION, "otr")
            .set(CUTLIST_VERSION, env!("CARGO_PKG_VERSION"))
            .set(CUTLIST_INTENDED_CUT_APP, "ffmpeg")
            .set(CUTLIST_NUM_OF_CUTS, format!("{}", self.len()))
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
        for i in 0..self.len() {
            // TODO: extract logic below in separate generic function?

            // Write frame interval
            if self.has_frame_intervals() {
                cutlist_ini
                    .with_section(Some(format!("{}{}", CUTLIST_CUT_SECTION, i)))
                    .set(
                        item_attr_start(BoundaryType::Frame),
                        format!("{}", self.frame_intervals.as_ref().unwrap()[i].from()),
                    )
                    .set(
                        item_attr_duration(BoundaryType::Frame),
                        format!(
                            "{}",
                            self.frame_intervals.as_ref().unwrap()[i].to()
                                - self.frame_intervals.as_ref().unwrap()[i].from()
                        ),
                    );
            }
            // Write time interval
            if self.has_time_intervals() {
                cutlist_ini
                    .with_section(Some(format!("{}{}", CUTLIST_CUT_SECTION, i)))
                    .set(
                        item_attr_start(BoundaryType::Time),
                        format!("{}", self.time_intervals.as_ref().unwrap()[i].from()),
                    )
                    .set(
                        item_attr_duration(BoundaryType::Time),
                        format!(
                            "{}",
                            self.time_intervals.as_ref().unwrap()[i].to()
                                - self.time_intervals.as_ref().unwrap()[i].from()
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
        if !self.has_frame_intervals() && !self.has_time_intervals() {
            return Err(anyhow!("Cut list does not contain intervals"));
        }

        fn validate_intervals<B>(intervals: &Vec<Interval<B>>) -> anyhow::Result<()>
        where
            B: Boundary,
        {
            let last_interval: Option<&Interval<B>> = None;

            for interval in intervals {
                if interval.from() > interval.to() {
                    return Err(anyhow!(
                        "Cut list intervals are invalid: From ({}) is after to ({})",
                        interval.from(),
                        interval.to()
                    ));
                }
                if let Some(last_interval) = last_interval {
                    if last_interval.to() > interval.from() {
                        return Err(anyhow!(
                            "Cut list intervals overlap: {} > {}",
                            last_interval.to(),
                            interval.from()
                        ));
                    }
                }
            }
            Ok(())
        }

        // If cut list contains time and frames intervals, both must have the
        // same number of items
        if self.has_frame_intervals()
            && self.has_time_intervals()
            && self.frame_intervals.as_ref().unwrap().len()
                != self.time_intervals.as_ref().unwrap().len()
        {
            return Err(anyhow!(
                "Cut list has time and frame intervals, but the numbers of intervals differ"
            ));
        }

        if self.has_frame_intervals() {
            validate_intervals(self.frame_intervals.as_ref().unwrap())
                .context("Frame intervals of cut list are invalid")?;
        }
        if self.has_time_intervals() {
            validate_intervals(self.time_intervals.as_ref().unwrap())
                .context("Time intervals of cut list are invalid")?;
        }

        Ok(())
    }
}
