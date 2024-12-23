// SPDX-FileCopyrightText: 2022-2024 Michael Picht <mipi@fsfe.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

mod cfg;
mod cli;
mod video;

use crate::video::Video;
use anyhow::anyhow;
use itertools::Itertools;
use log::*;
use regex::Regex;

/// Process videos (i.e., collect, move, decode and cut them). This is done in a
/// dedicated function (with appropriate result type) to be able to use the ?
/// operator to propagate errors
fn process_videos() -> anyhow::Result<()> {
    // Collect video files from command line parameters and (sub) working
    // directories. They are returned as vector sorted by video key and
    // (descending) status.
    #[allow(clippy::manual_try_fold)]
    #[allow(clippy::manual_inspect)]
    video::collect(&cli::videos())?
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
                info!("Processed already: \"{}\"", video.file_name());
            }
            video
        })
        // Decode videos and print error messages. Result of the closure is the
        // video (&mut Video), whether the decoding was successful or not. Errors
        // are collected in an attribute of the video structure
        .map(|video| {
            if cli::is_decode_command() || cli::is_process_command() {
                video.decode(cli::otr_access_data());
            }
            video
        })
        .map(|video| {
            if cli::is_cut_command() || cli::is_process_command() {
                video.cut(
                    cli::cutlist_access_type(),
                    if cli::is_cut_command() {
                        cli::cutlist_rating()
                    } else {
                        None
                    },
                    cli::min_cutlist_rating(),
                );
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
                error!("\"{}\":\n{:?}\n", video.file_name(), err);
                Err(anyhow!("An error occurred during processing of OTR videos"))
            } else {
                res
            }
        })
}

fn main() {
    // Set up logging (i.e., which messages are displayed on stdout and stderr)
    print_logger::new()
        // Only allow log messages from otr and its sub modules
        .targets_by_regex(&[Regex::new("^[otr|common|otr_utils][::.+]*").unwrap()])
        // Convert otr CLI flags into level filter of log
        .level_filter(if cli::quiet() {
            LevelFilter::Off
        } else {
            match cli::verbose() {
                0 => LevelFilter::Info,
                1 => LevelFilter::Debug,
                _ => LevelFilter::Trace,
            }
        })
        // Initialize logging
        .init()
        // Provoke dump in case of an error
        .unwrap();

    // Process video files (collect, decode and cut them)
    if process_videos().is_err() {
        std::process::exit(1);
    }
}
