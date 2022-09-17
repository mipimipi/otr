use cut::{cut, CutError};
use decode::decode;
use itertools::Itertools;
use rayon::prelude::*;
use video::Video;

mod cfg;
mod cut;
mod decode;
mod video;

// process (i.e., collect, move, decode and cut) videos
fn process_videos() -> anyhow::Result<()> {
    // collect video files from command line and (sub) working directories.
    // They are returned as vector sorted by video key and (descending) status
    (&mut video::collect_and_sort()?)
        .into_iter()
        // move video files to the working sub directories that correspond to
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
        // remove duplicate entries of the same video with "lower" status
        .dedup_by(|v1, v2| v1.key() == v2.key())
        // print message for already cut videos
        .map(|video| {
            video::nothing_to_do(video);
            video
        })
        // decode videos, receive result and print error messages. Result of
        // the closure is the video (&mut Video), whether the decoding was
        // successful or not
        .map(|video| match decode(video) {
            None => video,
            Some(err) => {
                eprintln!(
                    "{:?}",
                    err.context(format!("Could not decode {:?}", video.file_name()))
                );
                video
            }
        })
        // cut videos in parallel and print error messages. Result of
        // the closure is the video (&mut Video), whether the cutting was
        // successful or not
        .collect::<Vec<&mut Video>>()
        .into_par_iter()
        .map(|video| {
            if let Some(err) = cut(video) {
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
        // receive final results (i.e., results from cutting videos)
        .collect::<Vec<&mut Video>>();
    Ok(())
}

fn main() {
    // process video files (collect, decode and cut them)
    if let Err(err) = process_videos() {
        eprintln!("{:?}", err);
        std::process::exit(1);
    }
}
