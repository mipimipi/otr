use crate::{
    cli,
    video::cutlist::{self, CutList, Kind},
};
use anyhow::{anyhow, Context};
use log::*;
use std::{
    error::Error,
    fmt::{self, Debug, Display},
    path::Path,
    process::Command,
    str::{self, FromStr},
};

/// Special error type for cutting videos to be able to handle specific
/// situations - e.g., if no cutlist exists
#[derive(Debug, Default)]
pub enum CutError {
    Any(anyhow::Error),
    #[default]
    Default,
    NoCutlist,
}
/// Support the use of "{}" format specifier
impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CutError::Any(ref source) => write!(f, "Error: {}", source),
            CutError::Default => write!(f, "Default cut error"),
            CutError::NoCutlist => write!(f, "No cutlist exists"),
        }
    }
}
/// Support conversion of an anyhow::Error into a CutError
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
    // Call specialized cut functions: Either cut with a cut list derived from
    // the --intervals command line option or retrieve cut lists from a
    // provider
    if let cli::Commands::Cut {
        intervals: Some(intervals),
        ..
    } = &cli::args().command
    {
        cut_with_cli_intervals(in_path, out_path, intervals)
    } else if let cli::Commands::Cut {
        list: Some(cutlist_path),
        ..
    } = &cli::args().command
    {
        cut_with_cutlist_from_file(in_path, out_path, cutlist_path)
    } else {
        cut_with_cutlist_from_provider(in_path, out_path)
    }
}

/// Cut a video with a cut list derived from the --intervals command line
/// option. in_path is the path of the decoded video file.
/// out_path is the path of the cut video file.
fn cut_with_cli_intervals<P, Q, S>(in_path: P, out_path: Q, intervals: S) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    S: AsRef<str> + Display,
{
    let file_name = in_path.as_ref().file_name().unwrap().to_str().unwrap();
    let cutlist = CutList::from_str(intervals.as_ref())?;

    cutlist
        .validate()
        .context(format!("{} let to an invalid cut list", intervals))?;

    match cut_with_mkvmerge(&in_path, &out_path, &cutlist)
        .context(format!("Could not cut {:?} with {}", file_name, intervals))
    {
        Err(err) => Err(CutError::Any(err)),
        _ => Ok(()),
    }
}

/// Cut a video with a cut list read from an INI file. in_path is the path of the
/// decoded video file. out_path is the path of the cut video file.
fn cut_with_cutlist_from_file<P, Q, R>(
    in_path: P,
    out_path: Q,
    cutlist_path: R,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    R: AsRef<Path>,
{
    let file_name = in_path.as_ref().file_name().unwrap().to_str().unwrap();
    let cutlist = CutList::try_from(cutlist_path.as_ref())?;

    cutlist.validate().context(format!(
        "Cut list retrieved from '{}' is invalid",
        cutlist_path.as_ref().display()
    ))?;

    match cut_with_mkvmerge(&in_path, &out_path, &cutlist).context(format!(
        "Could not cut {:?} with cut list from '{}'",
        file_name,
        cutlist_path.as_ref().display()
    )) {
        Err(err) => Err(CutError::Any(err)),
        _ => Ok(()),
    }
}

/// Cut a video with a cut list retrieved from a provider. in_path is the path
/// of the decoded video file. out_path is the path of the cut video file.
fn cut_with_cutlist_from_provider<P, Q>(in_path: P, out_path: Q) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let file_name = in_path.as_ref().file_name().unwrap().to_str().unwrap();

    // Retrieve cut list headers from provider
    let headers: Vec<cutlist::ProviderHeader> = match cutlist::headers_from_provider(file_name)
        .context(format!("Could not retrieve cut lists for {:?}", file_name))
    {
        Ok(hdrs) => hdrs,
        _ => return Err(CutError::NoCutlist),
    };

    // Retrieve cut lists from provider and cut video
    let mut is_cut = false;
    for header in headers {
        match CutList::try_from(&header) {
            Ok(cutlist) => {
                cutlist.validate().context(format!(
                    "Cut list {} for {:?} is not valid",
                    header.id(),
                    file_name
                ))?;

                match cut_with_mkvmerge(&in_path, &out_path, &cutlist) {
                    Ok(_) => {
                        is_cut = true;
                        break;
                    }
                    Err(err) => {
                        error!(
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
                error!(
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

/// Cut a video file stored in in_path with mkvmerge using the given cut list.
/// The cut video is stored in out_path.
fn cut_with_mkvmerge<P, Q>(in_path: P, out_path: Q, cutlist: &CutList) -> anyhow::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    fn exec_mkvmerge<P, Q>(
        in_path: P,
        out_path: Q,
        kind: &Kind,
        cutlist: &CutList,
    ) -> anyhow::Result<()>
    where
        P: AsRef<Path>,
        Q: AsRef<Path>,
    {
        // Call mkvmerge to cut the video
        let output = Command::new("mkvmerge")
            .arg("-o")
            .arg(out_path.as_ref().to_str().unwrap())
            .arg("--split")
            .arg(cutlist.to_mkvmerge_split_str(kind)?)
            .arg(in_path.as_ref().to_str().unwrap())
            .output()?;
        if !output.status.success() {
            return Err(anyhow!(str::from_utf8(&output.stdout).unwrap().to_string()));
        }
        Ok(())
    }

    // Try all available kinds (frame numbers, time). After the cutting was
    // successful for one of them, exit
    let mut err = anyhow::Error::new(CutError::Default);
    for kind in cutlist.kinds() {
        if let Err(mkvmerge_err) =
            exec_mkvmerge(in_path.as_ref(), out_path.as_ref(), &kind, cutlist)
        {
            err = mkvmerge_err;
        } else {
            return Ok(());
        }
    }

    Err(err.context("mkvmerge returned an error"))
}
