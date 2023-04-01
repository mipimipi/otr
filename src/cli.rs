use clap::{Parser, Subcommand};
use indoc::indoc;
use once_cell::sync::OnceCell;
use std::path::PathBuf;

/// Structure to hold the command line arguments
#[derive(Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    version = env!("CARGO_PKG_VERSION"),
    author = env!("CARGO_PKG_AUTHORS"),
    about = env!("CARGO_PKG_DESCRIPTION")
)]
pub struct Args {
    #[command(subcommand)]
    pub command: Commands,
    #[arg(
        global = true,
        short = 'c',
        long = "config",
        help = "Path of config file (default is ~/.config/otr.json)"
    )]
    pub cfg_file_path: Option<PathBuf>,
    #[arg(
        global = true,
        short = 'd',
        long = "directory",
        help = "Working directory (overwrites config file content)"
    )]
    pub working_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(
        name = "cut",
        about = "Cut a video",
        long_about = indoc! {"
            Cut a video if possible (i.e., if either cutlists exist or cut intervals
            where submitted). The video files (the uncut and cut video files) are
            moved to the corresponding working (sub)directories
        "}
    )]
    Cut {
        #[arg(
            long = "intervals",
            value_name = "INTERVALS_STRING",
            help = indoc! {"
            Cut intervals, either as times or frames. The INTERVALS_STRING starts either
            with the key word \"frames\" or \"times\" depending on whether the video
            should be cut based on frame numbers or times. After a colon, the list of
            intervals must be specified as \"[<START>,<END>]...\". Times must be given as
            H:MM:SS.ssssss, where \"ssssss\" denotes the sub seconds part as nano seconds.
            This part is optional.
            Examples:
                \"times:[0:05:30,0:20:59.45]\"
                \"frames:[123,45667][48345,679868]\""}
        )]
        intervals: Option<String>,
        #[arg(name = "video", help = "Path of video to be cut")]
        video: PathBuf,
    },
    #[command(
        name = "process",
        about = "Decode and cut all videos",
        long_about = indoc! {"
            Decode and cut all videos which are either stored in the working
            (sub)directories or submitted as arguments. Videos are processed as far as
            possible (i.e., if there are no cutlists for a video, it will be cut, for
            example). Moreover, video files are moved to / stored in the corresponding
            working (sub) directories
        "}
    )]
    Process {
        #[arg(
            short = 'u',
            long = "user",
            help = "User name for Online TV Recorder (overwrites config file content)"
        )]
        user: Option<String>,
        #[arg(
            short = 'p',
            long = "password",
            help = "Password for Online TV Recorder (overwrites config file content)"
        )]
        password: Option<String>,
        videos: Vec<PathBuf>,
    },
}

/// Command line arguments. The conversion / determination into that structure
/// is done once only. The result is stored in a static variable.
pub fn args() -> &'static Args {
    static ARGS: OnceCell<Args> = OnceCell::new();
    ARGS.get_or_init(Args::parse)
}

impl Args {
    // TODO: avoid copying values
    pub fn videos(&self) -> Vec<PathBuf> {
        match &self.command {
            Commands::Cut { video, .. } => vec![video.to_path_buf()],
            Commands::Process { videos, .. } => videos.to_vec(),
        }
    }
}
