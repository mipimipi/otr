use super::{
    dirs::{self, DirKind},
    Video,
};

use anyhow::{anyhow, Context};
use log::*;
use std::{fs, path::Path};

/// Collects video files either from the submitted input paths, or (if no path
/// was submitted) from the working (sub) directories. The the corresponding
/// Video instances are created and returned as vector, sorted by key
/// (ascending) and status (descending).
pub fn collect(in_videos: &[&Path]) -> anyhow::Result<Vec<Video>> {
    let mut videos: Vec<Video> = Vec::new();

    // Collect videos from input array
    for path in in_videos {
        if let Ok(mut video) = Video::new(*path) {
            video.move_to_working_dir()?;
            videos.push(video);
            continue;
        }
        warn!("{:?} is not a valid video file: Ignored", path)
    }

    // If no videos have been submited: collect videos from working (sub)
    // directories
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
        info!("No videos found :(");
    } else {
        videos.sort();
    }

    Ok(videos)
}

/// Collect videos from the directory that is assigned to kind dir_kind
fn collect_videos_from_dir(dir_kind: &DirKind) -> anyhow::Result<Vec<Video>> {
    let mut videos: Vec<Video> = Vec::new();
    let dir = dirs::working_sub_dir(dir_kind)
        .context(format!("Could determine '{:?}' directory", &dir_kind))?;

    if !dir.is_dir() {
        return Err(anyhow!("{:?} is not a directory: Ignored", dir));
    }

    for file in
        fs::read_dir(dir).with_context(|| format!("Could not read '{:?}' directory", &dir_kind))?
    {
        let file_ref = file.as_ref().unwrap();

        if !file_ref.file_type()?.is_file() {
            continue;
        }

        match Video::new(file_ref.path().as_path()) {
            Ok(mut video) => {
                video.move_to_working_dir()?;
                videos.push(video);
            }
            Err(_) => {
                warn!("{:?} is not a valid video file: Ignored", &file_ref.path());
                continue;
            }
        }
    }

    Ok(videos)
}
