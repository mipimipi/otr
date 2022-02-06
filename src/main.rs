use cut::cut;
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
    video::collect_and_sort()?
        .into_iter()
        // move video files to the working sub directories that correspond to
        // their status
        .filter_map(
            move |video| match video::move_to_working_dir(video.clone()) {
                Ok(video) => Some(video),
                Err(err) => {
                    eprintln!(
                        "{:?}",
                        err.context(format!("Could not move {:?} to working directory", video))
                    );
                    None
                }
            },
        )
        // remove duplicate entries of the same video with "lower" status
        .dedup_by(|v1, v2| v1.key() == v2.key())
        // decode videos, receive result and print error messages. Result of
        // the closure is either the decoded video in case of success, or the
        // encoded video otherwise
        .map(|enc_video| match decode(&enc_video) {
            Ok(dec_video) => dec_video,
            Err(err) => {
                eprintln!(
                    "{:?}",
                    err.context(format!("Could not decode {:?}", enc_video.file_name()))
                );
                enc_video
            }
        })
        // cut videos in parallel and print error messages. Result type of
        // closure is anyhow::Result<Video>
        .collect::<Vec<Video>>()
        .par_iter()
        .map(|dec_video| match cut(dec_video) {
            Ok(cut_video) => Ok(cut_video),
            Err(err) => {
                let err = err.context(format!("Could not cut {:?}", dec_video.file_name()));
                eprintln!("{:?}", err);
                Err(err)
            }
        })
        // receive final results (i.e., results from cutting videos)
        .collect::<Vec<anyhow::Result<Video>>>();
    Ok(())
}

fn main() {
    // process video files (collect, decode and cut them)
    if let Err(err) = process_videos() {
        eprintln!("{:?}", err);
        std::process::exit(1);
    }
}
