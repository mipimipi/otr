use clap::{Parser, Subcommand};
use indoc::indoc;
use once_cell::sync::OnceCell;
use otr_utils::cutting::{CutlistAccessType, CutlistID, CutlistRating};
use std::path::{Path, PathBuf};

/// Converts cli parameters for cut list access into CutlistAccessType.
/// Note: Calling this function does only make sense for some sub commands.
///       If it is called when otr is called with a sub command that does
///       not have appropriate parameters, the function panics!
pub fn cutlist_access_type() -> CutlistAccessType<'static> {
    match &args().command {
        Commands::Cut {
            intervals,
            file,
            id,
            ..
        } => {
            if let Some(_intervals) = intervals {
                CutlistAccessType::Direct(_intervals)
            } else if let Some(_file) = file {
                CutlistAccessType::File(_file)
            } else if let Some(_id) = id {
                CutlistAccessType::ID(*_id)
            } else {
                CutlistAccessType::Auto
            }
        }
        Commands::Decode { .. } => {
            panic!("Sub command 'decode' does not have cut list access type as parameter")
        }
        Commands::Process { .. } => CutlistAccessType::Auto,
    }
}

/// Returns true if otr was called with sub command "cut", otherwise false
pub fn is_cut_command() -> bool {
    if let Commands::Cut { .. } = args().command {
        return true;
    }
    false
}

/// Returns true if otr was called with sub command "decode", otherwise false
pub fn is_decode_command() -> bool {
    if let Commands::Decode { .. } = args().command {
        return true;
    }
    false
}

/// Returns true if otr was called with sub command "process", otherwise false
pub fn is_process_command() -> bool {
    if let Commands::Process { .. } = args().command {
        return true;
    }
    false
}

/// Returns cut list rating
pub fn cutlist_rating() -> Option<CutlistRating> {
    match &args().command {
        Commands::Cut { rating, .. } => *rating,
        Commands::Decode { .. } => {
            panic!("Sub command 'decode' does not have cut list rating as parameter")
        }
        Commands::Process { .. } => {
            panic!("Sub command 'process' does not have cut list rating as parameter")
        }
    }
}

/// Returns minimum cut list rating
pub fn min_cutlist_rating() -> Option<CutlistRating> {
    match &args().command {
        Commands::Cut { min_rating, .. } => *min_rating,
        Commands::Decode { .. } => {
            panic!("Sub command 'decode' does not have minimum cut list rating as parameter")
        }
        Commands::Process { min_rating, .. } => *min_rating,
    }
}

/// Returns true if flag --quiet/-q was set by the user
pub fn quiet() -> bool {
    args().quiet
}

/// Returns the verbosity level that was set by the user
pub fn verbose() -> u8 {
    args().verbose
}

/// Returns videos (file paths) as array for different sub commands. This is
/// independent from number of videos a sub command required (i.e., if a sub
/// command required only one video, this video is returned in an array)
pub fn videos() -> Vec<&'static Path> {
    match &args().command {
        Commands::Cut { video, .. } => vec![video.as_path()],
        Commands::Decode { video, .. } => vec![video.as_path()],
        Commands::Process { videos, .. } => videos.iter().map(|p| p.as_path()).collect(),
    }
}

/// Returns OTR access data (user, password).
/// Note: Calling this function does only make sense for some sub commands.
///       If it is called when otr is called with a sub command that does
///       not have appropriate parameters, the function panics!
pub fn otr_access_data() -> Option<(&'static str, &'static str)> {
    match &args().command {
        Commands::Cut { .. } => {
            panic!("Sub command 'cut' does not have OTR access data as parameters")
        }
        Commands::Decode { user, password, .. } | Commands::Process { user, password, .. } => {
            // Note: Either both, user and password are Some(...) or None.
            //       This is ensured by the clap configuration
            if user.is_some() {
                Some((user.as_ref().unwrap(), password.as_ref().unwrap()))
            } else {
                None
            }
        }
    }
}

/// Structure to hold the command line arguments
#[derive(Parser)]
#[command(
    name = env!("CARGO_PKG_NAME"),
    version = env!("CARGO_PKG_VERSION"),
    author = env!("CARGO_PKG_AUTHORS"),
    about = env!("CARGO_PKG_DESCRIPTION")
)]
struct Args {
    #[command(subcommand)]
    command: Commands,
    #[arg(
        global = true,
        short = 'v',
        long = "verbose",
	action = clap::ArgAction::Count,
        help = indoc! {"
        Print status and progress information during command execution. The number of
        occurences of this flag defines the verbosity level. If this flag is not set at
        all, only error, warning and information messages are displayed. If it occurs
        once, debug messages are displayed. With two or more occurences the highest trace
        level is switched on"}
    )]
    verbose: u8,
    #[arg(
        global = true,
        short = 'q',
        long = "quiet",
        help = "Switch off output completely, even error messages will not be displayed"
    )]
    quiet: bool,
}

/// Command line arguments. The conversion into that structure is done once only.
/// The result is stored in a static variable.
fn args() -> &'static Args {
    static ARGS: OnceCell<Args> = OnceCell::new();
    ARGS.get_or_init(Args::parse)
}

/// (Sub) commands of otr command with its parameters and flags
#[derive(Subcommand)]
#[group(name = "input", required = false, multiple = false)]
enum Commands {
    #[command(
        name = "cut",
        about = "Cut a video",
        long_about = indoc! {"
            Cut a video if possible. That is the case if ...
              (a) at least one cut list exists on cutlist.at, which is either selected
                  automatically, or an ID of a cut list is submitted, or ...
              (b) or a cut list is given explicitly as sequence of intervals or as file.
 
            If the video was cut successfully, the corresponding files (i.e., the uncut
            and cut video files) are moved to the corresponding work (sub)directories"}
    )]
    Cut {
        #[arg(
            long = "cutlist",
            value_name = "intervals_string",
	    group = "input",
            help = indoc! {"
            Cut list as sequence of intervals, either based on time or frame numbers. The
            INTERVALS_STRING starts either with the key word \"frames\" or \"time\"
            respectively. After a colon, the list of intervals must be specified as
            \"[<START>,<END>]...\". Times must be given as [H...]H:MM:SS.ssssss, where
            \"ssssss\" denotes the sub seconds part as nano seconds. This part is
            optional.
            Examples:
                \"time:[0:05:30,0:20:59.45]\"
                \"frames:[123,45667][48345,679868]\""}
        )]
        intervals: Option<String>,
        #[arg(
            long = "cutlist-file",
            value_name = "path_of_cut_list_file",
	    group = "input",
            help = indoc! {"
            Path of a cut list file. The content of the file must have the INI format of
            cutlist.at"}
        )]
        file: Option<PathBuf>,
        #[arg(
            long = "cutlist-id",
            value_name = "cut_list_id",
            group = "input",
            help = "Identifier of a cut list at cutlist.at"
        )]
        id: Option<CutlistID>,
        #[arg(
            long = "min-rating",
            value_name = "min_cut_list_rating",
            group = "input",
            help = "Minímum rating a cut list must have for being used to cut the video"
        )]
        min_rating: Option<CutlistRating>,
        #[arg(
            long = "rating",
            value_name = "cut_list_rating",
            requires = "cutlist",
            help = "Rating of a self-created cut list"
        )]
        rating: Option<CutlistRating>,
        #[arg(name = "video", help = "Path of video to be cut")]
        video: PathBuf,
    },
    #[command(
        name = "decode",
        about = "Decode a video",
        long_about = indoc! {"
            Decode a video. The (decoded) video file is moved to the corresponding work
            (sub)directories"}
    )]
    Decode {
        #[arg(
            short = 'u',
            long = "user",
            help = "User name for Online TV Recorder (overwrites configuration file content)",
            requires("password")
        )]
        user: Option<String>,
        #[arg(
            short = 'p',
            long = "password",
            help = "Password for Online TV Recorder (overwrites configuration file content)",
            requires("user")
        )]
        password: Option<String>,
        #[arg(name = "video", help = "Path of video to be decoded")]
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
            help = "User name for Online TV Recorder (overwrites configuration file content)",
            requires("password")
        )]
        user: Option<String>,
        #[arg(
            short = 'p',
            long = "password",
            help = "Password for Online TV Recorder (overwrites configuration file content)",
            requires = "user"
        )]
        password: Option<String>,
        #[arg(
            long = "min-rating",
            value_name = "min_cut_list_rating",
            help = "Minímum rating a cut list must have for being used to cut the video"
        )]
        min_rating: Option<CutlistRating>,
        videos: Vec<PathBuf>,
    },
}
