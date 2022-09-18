use cut::{cut, CutError};
use decode::decode;
use itertools::Itertools;
use rayon::prelude::*;
use video::Video;

mod cfg;
mod cut;
mod decode;
mod video;

/// Process videos (i.e., collect, move, decode and cut them)
fn process_videos() -> anyhow::Result<()> {
    // Collect video files from command line parameters and (sub) working
    // directories. They are returned as vector sorted by video key and
    // (descending) status.
    video::collect_and_sort()?
        // Create an iterator that delivers type &mut Video
        .iter_mut()
        // Move video files to the working sub directories that correspond to
        // their status
        .filter_map(|video| match video::move_to_working_dir(video) {
            None => Some(video),
            Some(err) => {
                eprintln!(
                    "{:?}",
                    err.context(format!(
                        "Could not move {:?} to working directory",
                        video.as_ref()
                    ))
                );
                None
            }
        })
        // Remove duplicate entries of the same video with "lower" status.
        // I.e., if the same video (i.e., same key) exists, once in status
        // encoded and once in status decoded, the video with status encoded is
        // removed (just from the video vector, the video file is not removed).
        .dedup_by(|v1, v2| v1.key() == v2.key())
        // print message for already cut videos
        .map(|video| {
            if video.is_processed() {
                println!("Processed already: {:?}", video.file_name());
            }
            video
        })
        // Decode videos and print error messages. Result of the closure is the
        // video (&mut Video), whether the decoding was successful or not.
        .map(|video| match decode(video) {
            Ok(()) => video,
            Err(err) => {
                eprintln!(
                    "{:?}",
                    err.context(format!("Could not decode {:?}", video.file_name()))
                );
                video
            }
        })
        // Cut videos in parallel and print error messages. Result of
        // the closure is the video (&mut Video), whether the cutting was
        // successful or not.
        .collect::<Vec<&mut Video>>()
        .into_par_iter()
        .map(|video| {
            if let Err(err) = cut(video) {
                match err {
                    CutError::Any(err) => {
                        let err = err.context(format!("Could not cut {:?}", video.file_name()));
                        eprintln!("{:?}", err);
                    }
                    CutError::NoCutlist => {
                        println!("No cutlist exists for {:?}", video.file_name());
                    }
                }
            }
            video
        })
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
