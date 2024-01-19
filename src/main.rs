mod cli;
mod video;

use anyhow::anyhow;
use itertools::Itertools;
use log::*;
use rayon::prelude::*;
use stderrlog::{ColorChoice, LogLevelNum};
use video::Video;

/// Process videos (i.e., collect, move, decode and cut them). This is done in a
/// dedicated function (with appropriate result type) to be able to use the ?
/// operator to propagate errors
fn process_videos() -> anyhow::Result<()> {
    // Get OTR user and password parameters from command line (in case that was
    // submitted)
    let (user, password) = if cli::is_decode_command() || cli::is_process_command() {
        cli::args().otr_access_data()
    } else {
        (None, None)
    };

    // Collect video files from command line parameters and (sub) working
    // directories. They are returned as vector sorted by video key and
    // (descending) status.
    #[allow(clippy::manual_try_fold)]
    video::collect(&cli::args().videos())?
        // Create an iterator that delivers type &mut Video
        .iter_mut()
        // Remove duplicate entries of the same video with "lower" status.
        // I.e., if the same video (i.e., same key) exists, for example once in
        // status encoded and once in status decoded, the video with status
        // encoded is removed (just from the video vector, the video file is not
        // removed).
        .dedup_by(|v1, v2| v1.key() == v2.key())
        // Print message for already cut videos
        .map(|video| {
            if cli::is_process_command() && video.is_processed() {
                info!("Processed already: {:?}", video.file_name());
            }
            video
        })
        // Decode videos and print error messages. Result of the closure is the
        // video (&mut Video), whether the decoding was successful or not.
        .map(|video| {
            if cli::is_decode_command() || cli::is_process_command() {
                video.decode(user, password);
            }
            video
        })
        // Cut videos in parallel and print error messages. Result of
        // the closure is the video (&mut Video), whether the cutting was
        // successful or not.
        .collect::<Vec<&mut Video>>()
        .into_par_iter()
        .map(|video| {
            if cli::is_cut_command() || cli::is_process_command() {
                video.cut(cli::args().cutlist_access_type());
            }
            video
        })
        // Collect videos the parallel cut step
        .collect::<Vec<&mut Video>>()
        // Handle errors that occured during decoding or cutting. Because of the
        // collect step before, this is the standard fold and not the rayon fold
        .iter()
        .fold(Ok(()), |res, video| {
            if let Some(err) = video.error() {
                error!("{:?}", err);
                Err(anyhow!("An error occurred during processing of OTR videos"))
            } else {
                res
            }
        })
}

fn main() {
    // Set up logging (i.e., which messages are displayed on stdout and stderr)
    stderrlog::new()
        .show_level(false)
        .module(module_path!())
        .show_module_names(false)
        .color(ColorChoice::Auto)
        .quiet(cli::args().quiet)
        .verbosity(match cli::args().verbose {
            0 => LogLevelNum::Error,
            1 => LogLevelNum::Info,
            _ => LogLevelNum::Trace,
        })
        .init()
        .unwrap();

    // Process video files (collect, decode and cut them)
    if process_videos().is_err() {
        std::process::exit(1);
    }
}
