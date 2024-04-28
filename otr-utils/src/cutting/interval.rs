use anyhow::{anyhow, Context};
use lazy_static::lazy_static;
use regex::Regex;
use std::{
    fmt::{self, Debug, Display},
    ops::{Add, Sub},
    str::FromStr,
};

use super::info::Metadata;

// Regular expressions
lazy_static! {
    /// Reg exp for a string of intervals
    static ref RE_INTERVALS: Regex = Regex::new(r#"^(?<interval>\[[^\[\],]+,[^\[\],]+\])+$"#).unwrap();
    /// Reg exp for a string representing one interval
    static ref RE_INTERVAL: Regex = Regex::new(r#"^\[(?<from>[^\[\],]+),(?<to>[^\[\],]+)\]$"#).unwrap();
    /// Reg exp for the string representation of the time boundary of an interval
    static ref RE_TIME: Regex =
        Regex::new(r#"^(\d+):([0-5]\d)+:([0-5]\d)+(\.(\d{0,6}))*$"#).unwrap();
}

/// Generalization for interval boundary - i.e., a timestamp or frame number
pub trait Boundary:
    Clone
    + Copy
    + Debug
    + Display
    + FromStr<Err = anyhow::Error>
    + From<f64>
    + Add<Output = Self>
    + Sub<Output = Self>
    + PartialOrd
{
    fn to_frame(self, _: &Metadata) -> anyhow::Result<Frame>;
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

/// Conversion from string
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

/// Conversion from f64
impl From<f64> for Frame {
    fn from(frame: f64) -> Self {
        Frame(frame as usize)
    }
}

/// Conversion from string
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
    fn to_frame(self, _: &Metadata) -> anyhow::Result<Frame> {
        Ok(self)
    }

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
    fn from(secs: f64) -> Self {
        Time((secs * 1000000_f64) as u64)
    }
}
impl From<Time> for f64 {
    fn from(time: Time) -> Self {
        time.0 as f64 / 1000000_f64
    }
}

/// Conversion from string
impl FromStr for Time {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !RE_TIME.is_match(s) {
            return Err(anyhow!("\"{}\" is not a valid time string", s));
        }

        // Since here it is clear that s matches the regexp, we can use unwrap()
        // in the following safely

        let caps = RE_TIME.captures(s).unwrap();
        let hours = caps.get(1).unwrap().as_str().parse::<f64>().unwrap();
        let mins = caps.get(2).unwrap().as_str().parse::<f64>().unwrap();
        let secs = caps.get(3).unwrap().as_str().parse::<f64>().unwrap();

        Ok(Self::from(
            hours * 3600.0
                + mins * 60.0
                + secs
                + match caps.get(5) {
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

/// Conversion from string
impl<B> FromStr for Interval<B>
where
    B: Boundary,
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if RE_INTERVAL.is_match(s) {
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
        } else {
            Err(anyhow!("\"{}\" is not a valid interval", s))
        }
    }
}

impl<B> Interval<B>
where
    B: Boundary,
{
    pub fn from_from_to(from: B, to: B) -> Option<Self> {
        if from == to {
            None
        } else {
            Some(Interval::<B> { from, to })
        }
    }

    pub fn from_start_duration(start: B, duration: B) -> Option<Self> {
        if start == start + duration {
            None
        } else {
            Some(Interval::<B> {
                from: start,
                to: start + duration,
            })
        }
    }

    pub fn from(&self) -> B {
        self.from
    }

    pub fn to(&self) -> B {
        self.to
    }

    pub fn len(&self) -> B {
        self.to - self.from
    }

    pub fn to_frames(&self, metadata: &Metadata) -> anyhow::Result<Interval<Frame>> {
        Ok(Interval::<Frame> {
            from: self.from.to_frame(metadata)?,
            to: self.to.to_frame(metadata)?,
        })
    }

    pub fn to_times(&self, metadata: &Metadata) -> anyhow::Result<Interval<Time>> {
        Ok(Interval::<Time> {
            from: self.from.to_time(metadata)?,
            to: self.to.to_time(metadata)?,
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

pub fn intervals_from_str<B>(s: &str) -> anyhow::Result<Vec<Interval<B>>>
where
    B: Boundary,
{
    if RE_INTERVALS.is_match(s) {
        let mut intervals = vec![];

        // Split string in sub strings that contain a single interval string
        // each and create an interval fro it
        for s in s.split_inclusive(']').collect::<Vec<_>>() {
            let interval = Interval::<B>::from_str(s)?;
            if interval.len() > B::from(0_f64) {
                intervals.push(interval)
            }
        }

        Ok(intervals)
    } else {
        Err(anyhow!("\"{}\" is not a valid list of intervals", s))
    }
}
