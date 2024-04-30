use anyhow::{anyhow, Context};
use duct::cmd;
use lazy_static::lazy_static;
use log::*;
use regex::Regex;
use scopeguard::defer;
use std::{
    convert::Into,
    ffi::OsString,
    fmt::{self, Display},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output},
    str::{self, from_utf8},
};

use super::{
    info::Metadata,
    interval::{Boundary, Frame, Interval, Time},
};

const CUTTING_DIR_PREFIX: &str = "cutting";

/// File name template for interval files. This is done as macro instead of
/// &str constant since format strings must be literals, which is not possible
/// with a constant
/// Note: This macro must be consistent with the regular expression
///       RE_INTERNAL_FILE_NAME
macro_rules! interval_file_name_tmpl {
    () => {
        "part-{:03}-{}.{}"
    };
}

pub enum SubIntervalID {
    Pre,
    Main,
    Post,
}
impl Display for SubIntervalID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                SubIntervalID::Pre => 1,
                SubIntervalID::Main => 2,
                SubIntervalID::Post => 3,
            }
        )
    }
}

// Regular expressions
lazy_static! {
    // Name of a file that contains a part of the cut video.
    // Note: This regular expression must be consistent with the macro
    //       interval_file_name_tmpl
    static ref RE_INTERVAL_FILE_NAME: Regex =
        Regex::new(r"^part-\d{3}-\d\..*$").unwrap();
}

/// Checks if ffmpeg is properly installed. I.e., it is checked if it can be
/// called. If that is not possible, it could be that it is either not installed
/// or not in the path
pub fn is_installed() -> bool {
    match Command::new("ffmpeg").arg("-h").output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Cuts in_video based on cut intervals. Result is stored in out_video.
/// Temporary artefacts are stored in cutting_dir, a sub directory of tmp_dir.
/// Finally, cutting_dir is removed in any case
pub fn cut<I, O, T, B>(
    in_video: I,
    out_video: O,
    tmp_dir: T,
    intervals: std::slice::Iter<'_, Interval<B>>,
    metadata: &Metadata,
) -> anyhow::Result<()>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
    T: AsRef<Path>,
    B: Boundary,
{
    let err_msg = "Could not cut video with ffmpeg";

    // Create cutting directory for the video to be cut
    let cutting_dir = create_cutting_dir(&in_video, tmp_dir).context(err_msg)?;

    // Make sure the cutting directory is finally removed
    defer! {
        fs::remove_dir_all(cutting_dir.clone()).unwrap_or_else(|_| panic!("Cannot remove cutting directory"));
    trace!("Removed cutting directory \"{}\"", cutting_dir.display());
    }

    // zip() instead enumerate() is used because counting shall start with 1
    // instead of 0 due to better readability of file names and log messages
    for (i, interval) in (1..).zip(intervals) {
        trace!("Processing interval no {} ...", i);

        // Extract one interval from in_video and store the resulting files in
        // cutting_dir
        extract_interval(&in_video, &cutting_dir, metadata, interval, i).context(err_msg)?;
    }

    // Assemble out_video from the intermediate cut result stored in cutting_dir
    concatenate_intervals(&out_video, &cutting_dir).context(err_msg)?;

    Ok(())
}

/// Create a directory to store intermediate result files of re-encoding and
/// copying steps. This "cutting directory" is a sub directory of tmp_dir. Its
/// name is of the form "cutting_<FILE NAME OF THE TO-BE-CUT VIDEO>"
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
    fs::create_dir_all(&cutting_dir).context(format!(
        "Could not create cutting directory for \"{}\"",
        file_name
    ))?;

    trace!("Created cutting directory \"{}\"", cutting_dir.display());

    Ok(cutting_dir)
}

/// Extracs parts of in_video based on interval. interval is decomposed into up
/// to three sub intervals:
/// - First one is from the "from" boundary of the interval to the first key
///   frame that is in the interval
/// - Second one from the first key frame to the last key frame of the interval
/// - Third one is from the last key frame to the "to" boundary of the interval
/// The resutling video files are stored in cutting_dir. Their file names are of
/// the form "part-<INTERVAL_NO>-<SUB_INTERVAL_NO>. ...".
/// The second sub interval is just a copy, while the first and the third part
/// are done via re-encoding the corresponding parts of in_video.
/// If in_video has no key frames, the entire interval is just copied (without
/// doing any re-encoding).
/// If in_video has key frame, but there are no key frame in the interval, the
/// entire interval is re-encoded.
/// Since ffmpeg is used to copy and re-encode the sub intervals and ffmpeg
/// requires interval boundary as times, the (sub) intervals are finally
/// converted to (sub) intervals with time boundaries
fn extract_interval<I, C, B>(
    in_video: I,
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

    let err_msg = format!("Could not extract interval {}", interval);

    if metadata.has_frames() {
        // Turn interval into a frame interval. This must be done since we cannot
        // be sure that it is already a time interval
        let interval_f = interval.to_frames(metadata).context(err_msg.clone())?;
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
                    // before first key frame)
                    encode_interval(
                        &in_video,
                        &cutting_dir,
                        metadata,
                        interval.to_times(metadata)?,
                        interval_no,
                        SubIntervalID::Pre,
                    )
                    .context(err_msg.clone())?;
                }
            }

            // Copy main part of interval (from first to last key frame) -> sub
            // interval no = 2
            copy_interval(
                &in_video,
                &cutting_dir,
                interval_kf.to_times(metadata).context(err_msg.clone())?,
                interval_no,
            )
            .context(err_msg.clone())?;

            if interval_f.to() > interval_kf.to() {
                if let Some(interval) =
                    Interval::<Frame>::from_from_to(interval_kf.to() + 1, interval_f.to())
                {
                    // Re-encode postfix part of interval (from frame after key frame to
                    // original "to" frame
                    encode_interval(
                        &in_video,
                        &cutting_dir,
                        metadata,
                        interval.to_times(metadata).context(err_msg.clone())?,
                        interval_no,
                        SubIntervalID::Post,
                    )
                    .context(err_msg.clone())?;
                }
            }
        } else {
            // There is no key frame in the interval. Thus, the entire interval must
            // be re-encoded
            encode_interval(
                &in_video,
                &cutting_dir,
                metadata,
                interval.to_times(metadata).context(err_msg.clone())?,
                interval_no,
                SubIntervalID::Main,
            )
            .context(err_msg)?;
        }
    } else {
        // Copy entire cut interval. Since no (key) frames exist, the
        // frame-accurate approach above is not possible
        // As it is clear that interval is a time interval, we can safely use
        // unwrap()
        copy_interval(
            &in_video,
            &cutting_dir,
            interval.to_times(metadata).unwrap(),
            interval_no,
        )
        .context(err_msg)?;
    }

    trace!("Extracted interval");

    Ok(())
}

/// Copies interval from in_video and stores the result in cutting_dir
fn copy_interval<I, C>(
    in_video: I,
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
        in_video.as_ref().as_os_str(),
        "-c",
        "copy",
        cutting_dir
            .as_ref()
            .join(format!(
                interval_file_name_tmpl!(),
                interval_no,
                SubIntervalID::Main,
                in_video.as_ref().extension().unwrap().to_str().unwrap()
            ))
            .as_os_str()
    )
    .stdout_null()
    .stderr_capture()
    .unchecked()
    .run()
    .context(format!(
        "Could not execute ffmpeg to copy interval {}",
        interval
    ))?;

    if !output.status.success() {
        return Err(anyhow!("ffmpeg: {}", from_utf8(&output.stderr).unwrap())
            .context(format!("Could not copy interval {}", interval)));
    }

    trace!("Copied interval");

    Ok(())
}

/// (Re-)encodes interval from in_video and stores the result in cutting_dir
fn encode_interval<I, C>(
    in_video: I,
    cutting_dir: C,
    metadata: &Metadata,
    interval: Interval<Time>,
    interval_no: usize,
    sub_interval_id: SubIntervalID,
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
        in_video.as_ref().into(),
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
                sub_interval_id,
                in_video.as_ref().extension().unwrap().to_str().unwrap()
            ))
            .into(),
    );

    let output: Output = cmd("ffmpeg", args)
        .stdout_null()
        .stderr_capture()
        .unchecked()
        .run()
        .context("Could not execute ffmpeg to re-encode interval")?;

    if !output.status.success() {
        return Err(anyhow!("ffmpeg: {}", from_utf8(&output.stderr).unwrap())
            .context("Could not re-encode interval"));
    }

    trace!("Re-encoded interval");

    Ok(())
}

/// Concatenates all result files of re-encoding and copying steps
fn concatenate_intervals<O, C>(out_video: O, cutting_dir: C) -> anyhow::Result<()>
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
            fs::rename(cutting_dir.as_ref().join(&parts[0]), out_video).context(format!(
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
                out_video.as_ref().as_os_str()
            )
            .stdout_null()
            .stderr_capture()
            .unchecked()
            .run()
            .context("Could not execute ffmpeg to concatenate files")?;
            if !output.status.success() {
                Err(anyhow!("ffmpeg: {}", from_utf8(&output.stderr).unwrap())
                    .context("Could not concatenate files"))
            } else {
                Ok(())
            }
        }
    }
}
