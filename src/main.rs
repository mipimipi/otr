mod cli;
mod video;

use itertools::Itertools;
use rayon::prelude::*;
use video::Video;

/// Decodes a video if the current sub command required that
macro_rules! decode_or_not {
    ($video:ident) => {
        if cli::is_decode_command() || cli::is_process_command() {
            match $video.decode() {
                Ok(()) => $video,
                Err(err) => {
                    eprintln!(
                        "{:?}",
                        err.context(format!("Could not decode {:?}", $video.file_name()))
                    );
                    $video
                }
            }
        } else {
            $video
        }
    };
}

/// Cuts a video if the current sub command required that
macro_rules! cut_or_not {
    ($video:ident) => {
        if cli::is_cut_command() || cli::is_process_command() {
            match $video.cut() {
                Ok(()) => $video,
                Err(err) => {
                    eprintln!(
                        "{:?}",
                        err.context(format!("Could not cut {:?}", $video.file_name()))
                    );
                    $video
                }
            }
        } else {
            $video
        }
    };
}

/// Process videos (i.e., collect, move, decode and cut them)
fn process_videos() -> anyhow::Result<()> {
    // Collect video files from command line parameters and (sub) working
    // directories. They are returned as vector sorted by video key and
    // (descending) status.
    video::collect()?
        // Create an iterator that delivers type &mut Video
        .iter_mut()
        // Remove duplicate entries of the same video with "lower" status.
        // I.e., if the same video (i.e., same key) exists, for example once in
        // status encoded and once in status decoded, the video with status
        // encoded is removed (just from the video vector, the video file is not
        // removed).
        .dedup_by(|v1, v2| v1.key() == v2.key())
        // print message for already cut videos
        .map(|video| {
            if cli::is_process_command() && video.is_processed() {
                println!("Processed already: {:?}", video.file_name());
            }
            video
        })
        // Decode videos and print error messages. Result of the closure is the
        // video (&mut Video), whether the decoding was successful or not.
        .map(|video| decode_or_not!(video))
        // Cut videos in parallel and print error messages. Result of
        // the closure is the video (&mut Video), whether the cutting was
        // successful or not.
        .collect::<Vec<&mut Video>>()
        .into_par_iter()
        .map(|video| cut_or_not!(video))
        // Receive final results (i.e., results from cutting videos)
        .collect::<Vec<&mut Video>>();
    Ok(())
}

fn main() {
    // Process video files (collect, decode and cut them)
    if let Err(err) = process_videos() {
        eprintln!("{:?}", err);
        std::process::exit(1);
    }
}
