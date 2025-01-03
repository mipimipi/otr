// SPDX-FileCopyrightText: 2022-2024 Michael Picht <mipi@fsfe.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    dirs::{self, DirKind},
    Video,
};

use anyhow::{anyhow, Context};
use log::*;
use std::{env, fs, path::Path};

/// Collects video files either from the submitted input paths, or (if no path
/// was submitted) from the working (sub) directories. The corresponding Video
/// instances are created and returned as vector, sorted by key (ascending) and
/// status (descending).
pub fn collect(in_videos: &[&Path]) -> anyhow::Result<Vec<Video>> {
    let mut videos: Vec<Video> = Vec::new();

    // Collect videos from input array
    for path in in_videos {
        // Turn path into an absolute path
        let abs_path = if !path.is_absolute() {
            env::current_dir()?.join(path)
        } else {
            path.to_path_buf()
        };

        // Check if path exists
        if !abs_path.exists() {
            warn!("\"{}\" does not exist: Ignored", path.display());
            continue;
        }

        // Create video from abs_path. Since the path is canonicalized during
        // activity, it is not necessary to canonicalize it here
        if let Ok(mut video) = Video::new(&abs_path) {
            video.move_to_working_dir()?;
            videos.push(video);
            continue;
        }
        warn!("\"{}\" is not a valid video file: Ignored", path.display())
    }

    // If the function was called with an empty list of videos, collect videos from working (sub)
    // directories
    if in_videos.is_empty() {
        for dir_kind in [
            DirKind::Root,
            DirKind::Encoded,
            DirKind::Decoded,
            DirKind::Cut,
        ] {
            videos.append(&mut collect_videos_from_dir(&dir_kind).context(format!(
                "Could not retrieve videos from \"{}\" sub directory",
                &dir_kind
            ))?);
        }
    }

    if videos.is_empty() {
        info!("No videos to process");
    } else {
        videos.sort();
    }

    Ok(videos)
}

/// Collect videos from the directory that is assigned to kind dir_kind
fn collect_videos_from_dir(dir_kind: &DirKind) -> anyhow::Result<Vec<Video>> {
    let mut videos: Vec<Video> = Vec::new();
    let dir = dirs::working_sub_dir(dir_kind)
        .context(format!("Could determine \"{}\" directory", &dir_kind))?;

    if !dir.is_dir() {
        return Err(anyhow!("\"{}\" is not a directory: Ignored", dir.display()));
    }

    for file in
        fs::read_dir(dir).with_context(|| format!("Could not read \"{}\" directory", &dir_kind))?
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
                warn!(
                    "\"{}\" is not a valid video file: Ignored",
                    &file_ref.path().display()
                );
                continue;
            }
        }
    }

    Ok(videos)
}
