use super::{cfg, cfg::DirKind, Video};
use anyhow::{anyhow, Context};
use std::fs;

/// Collect video files from the working (sub) directories and from the paths
/// submitted via the command line, creates the corresponding Video instances
/// and returns them as vector, sorted by key (ascending) and status
/// (descending).
pub fn collect() -> anyhow::Result<Vec<Video>> {
    let mut videos: Vec<Video> = Vec::new();

    // collect videos from command line parameters
    for path in cfg::videos() {
        if let Ok(mut video) = Video::try_from(path) {
            video.move_to_working_dir()?;
            videos.push(video);
            continue;
        }
        println!("{:?} is not a valid video file: Ignored", path)
    }

    // if no videos have been submited via command line: collect videos from
    // working (sub) directories
    if videos.is_empty() {
        for dir_kind in [
            DirKind::Root,
            DirKind::Encoded,
            DirKind::Decoded,
            DirKind::Cut,
        ] {
            videos.append(&mut collect_videos_from_dir(&dir_kind).context(format!(
                "Could not retrieve videos from '{:?}' sub directory",
                &dir_kind
            ))?);
        }
    }

    if videos.is_empty() {
        println!("No videos found :(");
    } else {
        videos.sort();
    }

    Ok(videos)
}

/// Collect videos from the directory that is assigned to kind dir_kind
fn collect_videos_from_dir(dir_kind: &DirKind) -> anyhow::Result<Vec<Video>> {
    let mut videos: Vec<Video> = Vec::new();
    let dir = cfg::working_sub_dir(dir_kind)
        .context(format!("Could determine '{:?}' directory", &dir_kind))?;

    if !dir.is_dir() {
        return Err(anyhow!(format!("{:?} is not a directory: Ignored", dir)));
    }

    for file in
        fs::read_dir(dir).with_context(|| format!("Could not read '{:?}' directory", &dir_kind))?
    {
        let file_ref = file.as_ref().unwrap();

        if !file_ref.file_type()?.is_file() {
            continue;
        }

        match Video::try_from(&file_ref.path()) {
            Ok(mut video) => {
                video.move_to_working_dir()?;
                videos.push(video);
            }
            Err(_) => {
                println!("{:?} is not a valid video file: Ignored", &file_ref.path());
                continue;
            }
        }
    }

    Ok(videos)
}
