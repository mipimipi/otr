use anyhow::{anyhow, Context};
use lazy_static::lazy_static;
use regex::Regex;
use std::{
    fmt::{self, Debug, Display},
    ops::{Add, Sub},
    str::FromStr,
};

use super::info::Metadata;

/// Generalization of an interval boundary - i.e., a timestamp or frame number
pub trait Boundary:
    Clone
    + Copy
    + Display
    + FromStr<Err = anyhow::Error>
    + From<f64>
    + Into<f64>
    + Add<Output = Self>
    + Sub<Output = Self>
    + PartialOrd
{
    /// Convert boundary into frame number
    fn to_frame(self, _: &Metadata) -> anyhow::Result<Frame>;

    /// Convert boundary into timestamp
    fn to_time(self, _: &Metadata) -> anyhow::Result<Time>;
}

/// Boundary type
#[derive(Clone, Default, Eq, Hash, PartialEq)]
pub enum BoundaryType {
    #[default]
    Frame,
    Time,
}
impl Display for BoundaryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                BoundaryType::Frame => "frame",
                BoundaryType::Time => "time",
            }
        )
    }
}

/// Conversion from &str. Since a variety of different strings could be used to
/// indicate a timestamp or a frame number, the coding must take this into
/// account
impl FromStr for BoundaryType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.to_uppercase().contains("FRAME") {
            Ok(BoundaryType::Frame)
        } else if s.to_uppercase().contains("TIME") {
            Ok(BoundaryType::Time)
        } else {
            Err(anyhow!("\"{}\" is not a valid boundary type", s))
        }
    }
}

/// Wrapper type for frame numbers
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Frame(usize);
impl Display for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Conversion from and to usize
impl From<usize> for Frame {
    fn from(frame: usize) -> Self {
        Frame(frame)
    }
}
impl From<Frame> for usize {
    fn from(frame: Frame) -> Self {
        frame.0
    }
}

/// Conversion from and to f64
impl From<f64> for Frame {
    // frame is expected to not being negative. To be on the safe side, the
    // absolute value of frame is used
    fn from(frame: f64) -> Self {
        Frame(frame.abs() as usize)
    }
}
impl From<Frame> for f64 {
    fn from(frame: Frame) -> Self {
        frame.0 as f64
    }
}

/// Conversion from &str
impl FromStr for Frame {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s.parse::<f64>().context(format!(
            "Could not parse a frame number from \"{}\"",
            s
        ))?))
    }
}

/// Addition and substraction with same type and integer types
impl Add for Frame {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Frame(self.0 + rhs.0)
    }
}
impl Add<usize> for Frame {
    type Output = Self;

    fn add(self, rhs: usize) -> Self {
        Frame(self.0 + rhs)
    }
}

impl Sub for Frame {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Frame(self.0 - rhs.0)
    }
}
impl Sub<usize> for Frame {
    type Output = Self;

    fn sub(self, rhs: usize) -> Self {
        Frame(self.0 - rhs)
    }
}

impl Boundary for Frame {
    // Convert frame number into timestamp
    fn to_frame(self, _: &Metadata) -> anyhow::Result<Frame> {
        Ok(self)
    }

    // Conversion into timestamp: Nothing to do
    fn to_time(self, metadata: &Metadata) -> anyhow::Result<Time> {
        metadata.frame_to_time(self)
    }
}

/// Wrapper type for time (timestamps and time duration. Time is in microseconds
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Time(u64);
impl Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:.6}", self.0 as f64 / 1000000_f64)
    }
}

/// Conversion from and to f64 (f64 value is interpreted as time in seconds)
impl From<f64> for Time {
    // secs is expected to not being negative. To be on the safe side, the
    // absolute value of secs is used
    fn from(secs: f64) -> Self {
        Time((secs.abs() * 1000000_f64) as u64)
    }
}
impl From<Time> for f64 {
    fn from(time: Time) -> Self {
        time.0 as f64 / 1000000_f64
    }
}

lazy_static! {
    // Regular expression representing a time string
    static ref RE_TIME: Regex =
        Regex::new(r#"^(?<hours>\d+):(?<mins>[0-5]\d)+:(?<secs>[0-5]\d)+(\.(?<subs>\d{0,6}))*$"#).unwrap();
}

/// Conversion from &str
impl FromStr for Time {
    type Err = anyhow::Error;

    // s must match "[HH:MM:SS.ssssss]" with HH = hours, MM = minutes,
    // SS = seconds, ssssss = sub seconds
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !RE_TIME.is_match(s) {
            return Err(anyhow!("\"{}\" is not a valid time string", s));
        }

        // Since here it is clear that s matches the regexp, we can use unwrap()
        // in the following safely

        // Extract hours, minutes and seconds
        let caps = RE_TIME.captures(s).unwrap();
        let hours = caps.name("hours").unwrap().as_str().parse::<f64>().unwrap();
        if !(0.0..=23.0).contains(&hours) {
            return Err(anyhow!("Hours in {} are not valid", s));
        }
        let mins = caps.name("mins").unwrap().as_str().parse::<f64>().unwrap();
        let secs = caps.name("secs").unwrap().as_str().parse::<f64>().unwrap();

        Ok(Self::from(
            hours * 3600.0
                + mins * 60.0
                + secs
                + match caps.name("subs") {
                    Some(subs_match) => {
                        let subs_str = subs_match.as_str();
                        let subs = subs_str.parse::<f64>().unwrap();
                        subs * f64::powf(10_f64, -(subs_str.len() as f64))
                    }
                    None => 0.0,
                },
        ))
    }
}

/// Addition and subtraction for same type
impl Add for Time {
    type Output = Time;
    fn add(self, rhs: Self) -> Self {
        Time(self.0 + rhs.0)
    }
}
impl Sub for Time {
    type Output = Time;
    fn sub(self, rhs: Self) -> Self {
        Time(self.0 - rhs.0)
    }
}

impl Boundary for Time {
    // Convert timestamp into frame number
    fn to_frame(self, metadata: &Metadata) -> anyhow::Result<Frame> {
        if !metadata.has_frames() {
            Err(anyhow!(
                "Cannot turn time boundary into frame boundary, since video has no frames"
            ))
        } else {
            Ok(metadata.time_to_frame(self).context(format!(
                "Cannot turn time boundary {} into frame boundary",
                self
            ))?)
        }
    }

    // Conversion into timestamp: Nothing to do
    fn to_time(self, _: &Metadata) -> anyhow::Result<Time> {
        Ok(self)
    }
}

/// Generic cut interval
pub struct Interval<B>
where
    B: Boundary,
{
    from: B,
    to: B,
}
impl<B> Display for Interval<B>
where
    B: Boundary,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}, {}]", self.from, self.to)
    }
}

lazy_static! {
    /// Regular expression for a string representing one interval
    static ref RE_INTERVAL: Regex = Regex::new(r#"^\[(?<from>[^\[\],]+),(?<to>[^\[\],]+)\]$"#).unwrap();
}

/// Conversion from string
impl<B> FromStr for Interval<B>
where
    B: Boundary,
{
    type Err = anyhow::Error;

    /// s must have the form "[<FROM-STRING>,<TO-STRING>]", where FROM_STRING and
    /// TO_STRING must be according to the corresponding boundary type
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !RE_INTERVAL.is_match(s) {
            return Err(anyhow!("\"{}\" is not a valid interval", s));
        }

        Ok(Interval::<B> {
            from: B::from_str(
                RE_INTERVAL
                    .captures(s)
                    .unwrap()
                    .name("from")
                    .unwrap()
                    .as_str(),
            )?,
            to: B::from_str(
                RE_INTERVAL
                    .captures(s)
                    .unwrap()
                    .name("to")
                    .unwrap()
                    .as_str(),
            )?,
        })
    }
}

impl<B> Interval<B>
where
    B: Boundary,
{
    /// Creates an interval with from an to as boundaries. If required, from and
    /// to is switched to make sure that interval.form <= interval.to
    pub fn from_from_to(from: B, to: B) -> Self {
        if from < to {
            Interval::<B> { from, to }
        } else {
            Interval::<B> { from: to, to: from }
        }
    }

    /// Creates an interval with start as lower boundary and start + duration as
    /// upper boundary
    pub fn from_start_duration(start: B, duration: B) -> Self {
        Interval::<B> {
            from: start,
            to: start + duration,
        }
    }

    pub fn from(&self) -> B {
        self.from
    }

    pub fn to(&self) -> B {
        self.to
    }

    pub fn len(&self) -> f64 {
        Into::<f64>::into(self.to - self.from)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0.0
    }

    pub fn to_frames(&self, metadata: &Metadata) -> anyhow::Result<Interval<Frame>> {
        let err_msg = format!("Could not convert interval {} into frames", self);

        Ok(Interval::<Frame> {
            from: self.from.to_frame(metadata).context(err_msg.clone())?,
            to: self.to.to_frame(metadata).context(err_msg.clone())?,
        })
    }

    pub fn to_times(&self, metadata: &Metadata) -> anyhow::Result<Interval<Time>> {
        let err_msg = format!("Could not convert interval {} into times", self);

        Ok(Interval::<Time> {
            from: self.from.to_time(metadata).context(err_msg.clone())?,
            to: self.to.to_time(metadata).context(err_msg.clone())?,
        })
    }
}

impl Interval<Frame> {
    pub fn to_key_frames(&self, metadata: &Metadata) -> Option<Interval<Frame>> {
        if let Some(from) = metadata.key_frame_greater_or_equal_until_limit(self.from, self.to) {
            if let Some(to) = metadata.key_frame_less_or_equal_until_limit(self.to, self.from) {
                return Some(Interval::<Frame> { from, to });
            }
        }

        None
    }
}

lazy_static! {
    /// Regular expression for a string of intervals
    static ref RE_INTERVALS: Regex = Regex::new(r#"^(?<interval>\[[^\[\],]+,[^\[\],]+\])+$"#).unwrap();
}

/// Create a vector of intervals from a string representation of the form
/// "[<FROM-STRING>,<TO-STRING>][<FROM-STRING>,<TO-STRING>]..[<FROM-STRING>,<TO-STRING>]"
pub fn intervals_from_str<B>(s: &str) -> anyhow::Result<Vec<Interval<B>>>
where
    B: Boundary,
{
    if !RE_INTERVALS.is_match(s) {
        return Err(anyhow!("\"{}\" is not a valid list of intervals", s));
    }

    let mut intervals = vec![];

    // Split string into sub strings, where each sub string contains a single
    // interval string and create an interval from it
    for s in s.split_inclusive(']').collect::<Vec<_>>() {
        let interval = Interval::<B>::from_str(s)
            .context(format!("Could not convert \"{}\" into intervals", s))?;
        // Only accept intervals of length greater than zero
        if !interval.is_empty() {
            intervals.push(interval)
        }
    }

    Ok(intervals)
}
