use anyhow::{anyhow, Context};
use duct::cmd;
use lazy_static::lazy_static;
use log::*;
use regex::Regex;
use scopeguard::defer;
use std::{
    convert::Into,
    ffi::OsString,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    str::{self, from_utf8},
};

use super::{
    cutlist::Cutlist,
    info::Metadata,
    interval::{Boundary, Frame, Interval, Time},
};

const CUTTING_DIR_PREFIX: &str = "cutting";

/// File name template for interval files. This is done as macro instead of
/// &str constant since format string must be literals, which is not possible
/// with a constant
macro_rules! interval_file_name_tmpl {
    () => {
        "part-{:03}-{}.{}"
    };
}

// Regular expressions
lazy_static! {
    // Name of a file that contains a part of the cut video
    static ref RE_INTERVAL_FILE_NAME: Regex =
        Regex::new(r"^part-\d{3}-\d\..*$").unwrap();
}

/// Check if ffmpeg is properly installed. I.e., it is checked if it can be
/// called. If that is not possible, it could be that it is either not installed
/// or not in the path
pub fn is_installed() -> bool {
    match Command::new("ffmpeg").arg("-h").output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Cut a video file stored in in_path with ffmpeg using the given cut list.
/// The cut video is stored in out_path.
pub fn cut<I, O, T>(in_path: I, out_path: O, tmp_dir: T, cutlist: &Cutlist) -> anyhow::Result<()>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
    T: AsRef<Path>,
{
    trace!("Cutting video with ffmpeg ...");

    // Retrieve metadata of video to be cut
    let metadata = Metadata::new(&in_path)?;

    // Create cutting directory for the video to be cut
    let cutting_dir = create_cutting_dir(&in_path, tmp_dir)?;

    // Make sure the cutting directory is removed finally
    defer! {
        fs::remove_dir_all(cutting_dir.clone()).unwrap_or_else(|_| panic!("Cannot remove cutting directory for video \"{}\"",cutting_dir.display()));
    trace!("Removed cutting directory \"{}\"", cutting_dir.display());
    }

    // Try all available kinds (frame numbers, time). After the cutting was
    // successful for one of them, exit:

    // (1) Try cutting with frame intervals
    if cutlist.has_frame_intervals() {
        if !metadata.has_frames() {
            trace!("Since video has no frames, frame-based cut intervals cannot be used");
        } else if let Err(err) = cut_with_intervals(
            &in_path,
            &out_path,
            &cutting_dir,
            cutlist.frame_intervals()?,
            &metadata,
        ) {
            warn!(
                "Could not cut \"{}\" with frame intervals: {:?}",
                in_path.as_ref().display(),
                err
            );
        } else {
            trace!("Cut video with ffmpeg");

            return Ok(());
        }
    }

    // (2) Try cutting with time intervals
    if cutlist.has_time_intervals() {
        if let Err(err) = cut_with_intervals(
            &in_path,
            &out_path,
            &cutting_dir,
            cutlist.time_intervals()?,
            &metadata,
        ) {
            warn!(
                "Could not cut \"{}\" with time intervals: {:?}",
                in_path.as_ref().display(),
                err
            );
        } else {
            trace!("Cut video with ffmpeg");
            return Ok(());
        }
    }

    Err(anyhow!("Could not cut video with ffmpeg"))
}

fn create_cutting_dir<P, T>(path: P, tmp_dir: T) -> anyhow::Result<PathBuf>
where
    P: AsRef<Path>,
    T: AsRef<Path>,
{
    let file_name = path.as_ref().file_name().unwrap().to_str().unwrap();

    // Create cutting directory for the video to be cut
    let cutting_dir = tmp_dir
        .as_ref()
        .join(format!("{}-{}", CUTTING_DIR_PREFIX, file_name));
    fs::create_dir_all(&cutting_dir)
        .with_context(|| format!("Could not create cutting directory for \"{}\"", file_name))?;

    trace!("Created cutting directory \"{}\"", cutting_dir.display());

    Ok(cutting_dir)
}

fn cut_with_intervals<I, O, C, B>(
    in_path: I,
    out_path: O,
    cutting_dir: C,
    intervals: std::slice::Iter<'_, Interval<B>>,
    metadata: &Metadata,
) -> anyhow::Result<()>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
    C: AsRef<Path>,
    B: Boundary,
{
    // zip() instead enumerate() is used because counting shall start with 1
    // instead of 0 due to better readability of file names and log messages
    for (i, interval) in (1..).zip(intervals) {
        trace!("Processing interval no {} ...", i);

        extract_interval(&in_path, &cutting_dir, metadata, interval, i)?;
    }

    concatenate_intervals(&out_path, &cutting_dir)?;

    Ok(())
}

fn extract_interval<I, C, B>(
    in_path: I,
    cutting_dir: C,
    metadata: &Metadata,
    interval: &Interval<B>,
    interval_no: usize,
) -> anyhow::Result<()>
where
    I: AsRef<Path>,
    C: AsRef<Path>,
    B: Boundary,
{
    trace!("Extracting interval {} ...", interval);

    if metadata.has_frames() {
        // Turn interval into a frame interval. This must be done since we cannot
        // be sure that it is already a time interval
        let interval_f = interval.to_frames(metadata)?;
        trace!("Converted interval to frames: {}", interval_f);

        // Try turning frame interval into interval with key frames as
        // boundaries
        if let Some(interval_kf) = interval_f.to_key_frames(metadata) {
            trace!("Converted interval to key frames: {}", interval_kf,);

            if interval_f.from() < interval_kf.from() {
                if let Some(interval) =
                    Interval::<Frame>::from_from_to(interval_f.from(), interval_kf.from() - 1)
                {
                    // Re-encode prefix part of interval (from original "from" frame to frame
                    // before first key frame) -> segment = 1
                    encode_interval(
                        &in_path,
                        &cutting_dir,
                        metadata,
                        interval.to_times(metadata)?,
                        interval_no,
                        1,
                    )?;
                }
            }

            // Copy main part of interval (from first to last key frame) -> segment = 2
            copy_interval(
                &in_path,
                &cutting_dir,
                interval_kf.to_times(metadata)?,
                interval_no,
            )?;

            if interval_f.to() > interval_kf.to() {
                if let Some(interval) =
                    Interval::<Frame>::from_from_to(interval_kf.to() + 1, interval_f.to())
                {
                    // Re-encode postfix part of interval (from frame after key frame to
                    // original "to" frame -> segment = 3
                    encode_interval(
                        &in_path,
                        &cutting_dir,
                        metadata,
                        interval.to_times(metadata)?,
                        interval_no,
                        3,
                    )?;
                }
            }
        } else {
            // There is no key frame in the interval. Thus, the entire interval must
            // be re-encoded -> segment = 2
            encode_interval(
                &in_path,
                &cutting_dir,
                metadata,
                interval.to_times(metadata)?,
                interval_no,
                2,
            )?;
        }
    } else {
        // Copy entire cut interval. Since no (key) frames exist, the
        // frame-accurate approach above is not possible -> segment = 2
        // As it is clear that interval is a time interval, we can safely use
        // unwrap()
        copy_interval(
            &in_path,
            &cutting_dir,
            interval.to_times(metadata).unwrap(),
            interval_no,
        )?;
    }

    trace!("Extracted interval");

    Ok(())
}

fn copy_interval<I, C>(
    in_path: I,
    cutting_dir: C,
    interval: Interval<Time>,
    interval_no: usize,
) -> anyhow::Result<()>
where
    I: AsRef<Path>,
    C: AsRef<Path>,
{
    trace!("Copying interval {} ...", interval);

    // Copy an interval from video file with ffmpeg
    let output: Output = cmd!(
        "ffmpeg",
        "-ss",
        format!("{}", interval.from()),
        "-t",
        format!("{}", interval.len()),
        "-i",
        in_path.as_ref().as_os_str(),
        "-c",
        "copy",
        cutting_dir
            .as_ref()
            .join(format!(
                interval_file_name_tmpl!(),
                interval_no,
                2,
                in_path.as_ref().extension().unwrap().to_str().unwrap()
            ))
            .as_os_str()
    )
    .stdout_null()
    .stderr_capture()
    .unchecked()
    .run()
    .context("Could not execute ffmpeg to copy interval")?;

    if output.status.success() {
        trace!("Copied interval");
        Ok(())
    } else {
        Err(anyhow!("ffmpeg: {}", from_utf8(&output.stderr).unwrap())
            .context("Could not copy interval"))
    }
}

fn encode_interval<I, C>(
    in_path: I,
    cutting_dir: C,
    metadata: &Metadata,
    interval: Interval<Time>,
    interval_no: usize,
    segment: usize,
) -> anyhow::Result<()>
where
    I: AsRef<Path>,
    C: AsRef<Path>,
{
    trace!("Re-encoding interval {} ...", interval);

    // Assemble arguments for call of ffmpeg
    let mut args: Vec<OsString> = vec![
        "-ss".into(),
        format!("{}", interval.from()).into(),
        "-t".into(),
        format!("{}", interval.len()).into(),
        "-i".into(),
        in_path.as_ref().into(),
    ];
    for stream in metadata.streams() {
        args.push(format!("-c:{}", stream.index()).into());
        let encoding: OsString = if stream.codec().is_none() {
            "copy".to_string()
        } else {
            stream.codec().unwrap()
        }
        .into();
        args.push(encoding);
    }
    args.push(
        cutting_dir
            .as_ref()
            .join(format!(
                interval_file_name_tmpl!(),
                interval_no,
                segment,
                in_path.as_ref().extension().unwrap().to_str().unwrap()
            ))
            .into(),
    );

    let output: Output = cmd("ffmpeg", args)
        .stdout_null()
        .stderr_capture()
        .unchecked()
        .run()
        .context("Could not execute ffmpeg to re-encode interval")?;

    if output.status.success() {
        trace!("Re-encoded interval");
        Ok(())
    } else {
        Err(anyhow!("ffmpeg: {}", from_utf8(&output.stderr).unwrap())
            .context("Could not re-encode interval"))
    }
}

fn concatenate_intervals<O, C>(out_path: O, cutting_dir: C) -> anyhow::Result<()>
where
    O: AsRef<Path>,
    C: AsRef<Path>,
{
    trace!("Concatenating interval files ...");

    // Read relative paths of interval files into a vector and sort it for
    // further processing
    let mut parts: Vec<_> = fs::read_dir(&cutting_dir)
        .context(format!(
            "Cannot read cutting directory \"{}\"",
            cutting_dir.as_ref().display()
        ))?
        .filter_map(|entry| {
            if let Ok(entry) = entry {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        let file_name = entry.file_name().into_string().unwrap();
                        if RE_INTERVAL_FILE_NAME.is_match(&file_name) {
                            debug!("Found interval file \"{}\"", entry.path().display());
                            Some(file_name)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                error!(
                    "Error reading entry from cutting directory \"{}\"",
                    cutting_dir.as_ref().display()
                );
                None
            }
        })
        .collect();
    parts.sort();

    // Copy or concatenate interval files to result file
    match parts.len() {
        0 => {
            // No interval files found: There must be something wrong
            Err(anyhow!("No interval files found. Nothing to concatenate"))
        }
        1 => {
            // There is only one interval file. Thus, no concatenation required.
            // The file is taken as the cutting result
            fs::rename(cutting_dir.as_ref().join(&parts[0]), out_path).context(format!(
                "Cannot move cut internal \"{}\" to cut directory",
                &parts[0]
            ))?;

            Ok(())
        }
        _ => {
            // There are multiple interval files. These must be concatenated with
            // ffmpeg

            // Write relative paths of interval files to an index file
            let index_file_path = cutting_dir.as_ref().join("index.txt");
            let mut index_file: File =
                File::create(&index_file_path).context("Cannot create index file")?;
            for file_name in parts {
                writeln!(index_file, "file '{}'", file_name)
                    .context(format!("Cannot write \"{}\" to index file", file_name))?;
            }

            // Execute ffmpeg to concatenate partial cut files
            // Note: If absolute paths were used, ffmpeg would have to be called with
            //       "-safe 0"
            let output: Output = cmd!(
                "ffmpeg",
                "-f",
                "concat",
                "-i",
                index_file_path.as_os_str(),
                "-c",
                "copy",
                out_path.as_ref().as_os_str()
            )
            .stdout_null()
            .stderr_capture()
            .unchecked()
            .run()
            .context("Could not execute ffmpeg to concatenate files")?;
            if output.status.success() {
                Ok(())
            } else {
                Err(anyhow!("ffmpeg: {}", from_utf8(&output.stderr).unwrap())
                    .context("Could not concatenate files"))
            }
        }
    }
}
