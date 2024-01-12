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
        help = indoc! {"
        Path of configuration file (default is ~/.config/otr.json on Linux and
        ~/Library/Application\\ Support/otr.json on macOS)"}
    )]
    pub cfg_file_path: Option<PathBuf>,
    #[arg(
        global = true,
        short = 'd',
        long = "directory",
        help = "Working directory (overwrites configuration file content)"
    )]
    pub working_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
#[group(name = "input", required = false, multiple = false)]
pub enum Commands {
    #[command(
        name = "cut",
        about = "Cut a video",
        long_about = indoc! {"
            Cut a video if possible (i.e., either at least one cut list exists on cutlist.at,
            a cut list file is given, or cut intervals are submitted). 
            The video files (the uncut and cut video files) are moved to the corresponding
            work (sub)directories"}
    )]
    Cut {
        #[arg(
	    short = 'i',
            long = "intervals",
            value_name = "INTERVALS_STRING",
	    group = "input",
            help = indoc! {"
            Cut intervals, either based on time or frames numbers. The INTERVALS_STRING
            starts either with the key word \"frames\" or \"time\" respectively. After a
            colon, the list of intervals must be specified as \"[<START>,<END>]...\".
            Times must be given as [H...]H:MM:SS.ssssss, where \"ssssss\" denotes the sub
            seconds part as nano seconds. This part is optional.
            Examples:
                \"time:[0:05:30,0:20:59.45]\"
                \"frames:[123,45667][48345,679868]\""}
        )]
        intervals: Option<String>,
        #[arg(
	    short = 'l',
            long = "list",
            value_name = "CUTLIST_FILE_PATH",
	    group = "input",
            help = indoc! {"
            Path of a cut list file. The content of the file must have the INI format of
            cutlist.at."}
        )]
        list: Option<PathBuf>,
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
            help = "User name for Online TV Recorder (overwrites configuration file content)"
        )]
        user: Option<String>,
        #[arg(
            short = 'p',
            long = "password",
            help = "Password for Online TV Recorder (overwrites configuration file content)"
        )]
        password: Option<String>,
        videos: Vec<PathBuf>,
    },
}

/// Command line arguments. The conversion into that structure is done once only.
// The result is stored in a static variable.
pub fn args() -> &'static Args {
    static ARGS: OnceCell<Args> = OnceCell::new();
    ARGS.get_or_init(Args::parse)
}

impl Args {
    /// Returns videos (file paths) as array for different sub commands. This is
    /// independent from number of videos a sub command required (i.e., only one
    /// or many)
    pub fn videos(&self) -> Vec<PathBuf> {
        // TODO: avoid copying values
        match &self.command {
            Commands::Cut { video, .. } => vec![video.to_path_buf()],
            Commands::Process { videos, .. } => videos.to_vec(),
        }
    }
}
