use crate::{
    cli,
    video::cutlist::{self, CutList},
};
use anyhow::{anyhow, Context};
use std::{
    error::Error,
    fmt::{self, Debug, Display},
    path::Path,
    process::Command,
    str::{self, FromStr},
};

/// Special error type for cutting videos to be able to handle specific
/// situations - e.g., if no cutlist exists
#[derive(Debug)]
pub enum CutError {
    Any(anyhow::Error),
    NoCutlist,
}
/// Support the use of "{}" format specifier
impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CutError::Any(ref source) => write!(f, "Error: {}", source),
            CutError::NoCutlist => write!(f, "No cutlist exists"),
        }
    }
}
/// Support conversion an Error into a CutError
impl Error for CutError {}
/// Support conversion of an anyhow::Error into CutError
impl From<anyhow::Error> for CutError {
    fn from(err: anyhow::Error) -> CutError {
        CutError::Any(err)
    }
}

/// Cut a decoded video file. in_path is the path of the decoded video file.
/// out_path is the path of the cut video file.
pub fn cut<P, Q>(in_path: P, out_path: Q) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    if let cli::Commands::Cut {
        intervals: Some(intervals),
        ..
    } = &cli::args().command
    {
        cut_with_cli_cutlist(in_path, out_path, intervals)
    } else {
        cut_with_provider_cutlist(in_path, out_path)
    }
}

fn cut_with_cli_cutlist<P, Q, S>(in_path: P, out_path: Q, intervals: S) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    S: AsRef<str> + Display,
{
    let file_name = in_path.as_ref().file_name().unwrap().to_str().unwrap();
    let cutlist = CutList::from_str(intervals.as_ref())?;

    if !cutlist.is_valid() {
        return Err(CutError::Any(anyhow!(
            "{} let to an invalid cut list",
            intervals
        )));
    }

    match cut_with_mkvmerge(&in_path, &out_path, &cutlist)
        .context(format!("Could not cut {:?} with {}", file_name, intervals))
    {
        Err(err) => Err(CutError::Any(err)),
        _ => Ok(()),
    }
}

fn cut_with_provider_cutlist<P, Q>(in_path: P, out_path: Q) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let file_name = in_path.as_ref().file_name().unwrap().to_str().unwrap();

    // retrieve cutlist headers
    let headers: Vec<cutlist::ProviderHeader> = match cutlist::headers_from_provider(file_name)
        .context(format!("Could not retrieve cut lists for {:?}", file_name))
    {
        Ok(hdrs) => hdrs,
        _ => return Err(CutError::NoCutlist),
    };

    // retrieve cutlists and cut video
    let mut is_cut = false;
    for header in headers {
        match CutList::try_from(&header) {
            Ok(cutlist) => {
                if !cutlist.is_valid() {
                    return Err(CutError::Any(anyhow!(
                        "Cut list {} for {:?} is not valid",
                        header.id(),
                        file_name
                    )));
                }

                // cut video with mkvmerge
                match cut_with_mkvmerge(&in_path, &out_path, &cutlist) {
                    Ok(_) => {
                        // exit loop since video is cut
                        is_cut = true;
                        break;
                    }
                    Err(err) => {
                        eprintln!(
                            "{:?}",
                            anyhow!(err).context(format!(
                                "Could not cut {:?} with cut list {}",
                                file_name,
                                header.id()
                            ))
                        );
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "{:?}",
                    anyhow!(err).context(format!(
                        "Could not retrieve cut list {} for {:?}",
                        header.id(),
                        file_name
                    ))
                );
            }
        }
    }

    if !is_cut {
        return Err(CutError::Any(anyhow!(
            "No cut list could be successfully applied to cut {:?}",
            file_name
        )));
    }

    Ok(())
}

/// Cut a video file stored in in_path with mkvmerge using the cutlist
/// information in header and items and stores the cut video in out_path.
fn cut_with_mkvmerge<P, Q>(in_path: P, out_path: Q, cutlist: &CutList) -> anyhow::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    // call mkvmerge to cut the video
    let output = Command::new("mkvmerge")
        .arg("-o")
        .arg(out_path.as_ref().to_str().unwrap())
        .arg("--split")
        .arg(cutlist.to_mkvmerge_split_str())
        .arg(in_path.as_ref().to_str().unwrap())
        .output()?;
    if !output.status.success() {
        return Err(anyhow!(str::from_utf8(&output.stdout).unwrap().to_string())
            .context("mkvmerge returned an error"));
    }

    Ok(())
}
