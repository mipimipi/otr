use anyhow::anyhow;
use log::*;
use std::{fmt::Write, path::Path, process::Command, str};

use super::cutlist::{Cutlist, Kind};

/// Check if mkvmerge is properly installed. I.e., it is checked if it can be
/// called. If that is not possible, it could be that it is either not installed
/// or not in the path
pub fn is_installed() -> bool {
    match Command::new("mkvmerge").arg("-V").output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Cut a video file stored in in_path with mkvmerge using the given cut list.
/// The cut video is stored in out_path.
pub fn cut<P, Q>(in_path: P, out_path: Q, cutlist: &Cutlist) -> anyhow::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    // Try all available kinds (frame numbers, time). After the cutting was
    // successful for one of them, exit
    let mut err = anyhow::Error::msg("Dummy error");
    for kind in cutlist.kinds() {
        if let Err(_err) = exec_mkvmerge(&in_path, &out_path, kind, cutlist) {
            warn!(
                "MKVMerge could not cut \"{}\" with cut list of kind \"{}\"",
                in_path.as_ref().display(),
                kind
            );
            err = _err;
        } else {
            return Ok(());
        }
    }

    Err(err.context("Could not cut video with mkvmerge"))
}

/// Execute mkvmerge command to cut a video
fn exec_mkvmerge<P, Q>(
    in_path: P,
    out_path: Q,
    kind: &Kind,
    cutlist: &Cutlist,
) -> anyhow::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let output = Command::new("mkvmerge")
        .arg("-o")
        .arg(out_path.as_ref().to_str().unwrap())
        .arg("--split")
        .arg(to_split_str(cutlist, kind)?)
        .arg(in_path.as_ref().to_str().unwrap())
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(str::from_utf8(&output.stdout).unwrap().to_string()))
    }
}

/// Convert the floating point representation of an interval start or end of a
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

/// Create the split string that mkvmerge requires to cut a video from a cut list
pub fn to_split_str(cutlist: &Cutlist, kind: &Kind) -> anyhow::Result<String> {
    if !cutlist.is_of_kind(kind) {
        return Err(anyhow!(
            "Cannot create mkvmerge split string: Cut list does not contain \"{}\"\" intervals",
            kind
        ));
    }

    let mut split_str = match kind {
        Kind::Frames => "parts-frames:",
        Kind::Time => "parts:",
    }
    .to_string();

    for (i, item) in cutlist.items(kind)?.enumerate() {
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
