mod cutlist;

pub use cutlist::{
    AccessType as CutlistAccessType, Ctrl as CutlistCtrl, Rating as CutlistRating, ID as CutlistID,
};

use anyhow::{anyhow, Context};
use cutlist::{Cutlist, Kind};
use log::*;
use std::{
    error::Error,
    fmt::{self, Debug, Display},
    path::Path,
    process::Command,
    str,
};

/// Special error type for cutting videos to be able to handle specific
/// situations - e.g., if no cutlist exists
#[derive(Debug, Default)]
pub enum CutError {
    Any(anyhow::Error),
    #[default]
    Default,
    NoCutlist,
    CutlistSubmissionFailed(anyhow::Error),
}
/// Support the use of "{}" format specifier
impl fmt::Display for CutError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            CutError::Any(ref source) => write!(f, "Error: {}", source),
            CutError::Default => write!(f, "Default cut error"),
            CutError::NoCutlist => write!(f, "No cutlist exists"),
            CutError::CutlistSubmissionFailed(ref source) => {
                write!(f, "Submission of cut list to cutlist.at failed: {}", source)
            }
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

/// Check if mkvmerge can be called. If that is not possible, it could be that
/// it is either not installed or not in the path
pub fn check_mkvmerge() -> anyhow::Result<()> {
    let output = Command::new("mkvmerge")
        .arg("-V")
        .output()
        .context("Something went wrong when calling mkvmerge")?;

    if output.status.success() {
        Ok(())
    } else {
        let err_msg: String = str::from_utf8(&output.stderr)
            .unwrap_or_else(|_| panic!("Cannot extract error message from mkvmerge call"))
            .to_string();
        Err(anyhow!("mkvmerge cannot be called").context(err_msg))
    }
}

/// Cut a decoded video file.
/// - in_path is the path of the decoded video file. out_path is the path of the
///   to-be-cut video file
/// - out_path is the path of resulting file
/// - tmp_dir is the directory where OTR stores the cut list (provided a cut list
///   file is genererated and uploaded to cutlist.at)
/// - cutlist_ctrl contains attributes to control handling of cut lists, such as
///   - access_type: specifies how to (try to) get an appropriate cut list
///   - min_rating: specifies the minimum rating a cutlist must have when
///     automatically selected from the cut list provider
///   - submit: whether cut list shall shall be uploaded to cutlist.at. In this
///     case an access token is required. Submitting cut lists does only make
///     sense if a video is cut based on intervals
///   - rating: rating of the to-be-uploaded cut lists (overwriting the default
///     which is defined in the configuration file)
pub fn cut<P, Q, T>(
    in_path: P,
    out_path: Q,
    tmp_dir: T,
    cutlist_ctrl: &CutlistCtrl,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    T: AsRef<Path>,
{
    // Call specialized cut functions based on the cut list access type that was
    // submitted
    match cutlist_ctrl.access_type {
        cutlist::AccessType::Direct(intervals) => cut_with_cutlist_from_intervals(
            in_path,
            out_path,
            tmp_dir,
            intervals,
            cutlist_ctrl.submit,
            cutlist_ctrl.access_token,
            cutlist_ctrl.rating,
        ),
        cutlist::AccessType::File(file) => cut_with_cutlist_from_file(in_path, out_path, file),
        cutlist::AccessType::ID(id) => cut_with_cutlist_from_provider_by_id(in_path, out_path, id),
        _ => cut_with_cutlist_from_provider_auto_select(in_path, out_path, cutlist_ctrl.min_rating),
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
    trace!(
        "Cutting \"{}\" with cut list from \"{}\"",
        in_path.as_ref().display(),
        cutlist_path.as_ref().display()
    );

    let cutlist = Cutlist::try_from(cutlist_path.as_ref())?;

    match cut_with_mkvmerge(&in_path, &out_path, &cutlist).context(format!(
        "Could not cut video with cut list from '{}'",
        cutlist_path.as_ref().display()
    )) {
        Err(err) => Err(CutError::Any(err)),
        _ => Ok(()),
    }
}

/// Cut a video with a cut list derived from an intervals string. in_path is the
/// path of the decoded video file. out_path is the path of the cut video file.
/// submit_cutlists defines whether cut lists are submitted to cutlist.at. In
/// this case an access token is required
fn cut_with_cutlist_from_intervals<P, Q, T, I>(
    in_path: P,
    out_path: Q,
    tmp_dir: T,
    intervals: I,
    submit_cutlists: bool,
    cutlist_at_access_token: Option<&str>,
    rating: CutlistRating,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    T: AsRef<Path>,
    I: AsRef<str> + Display,
{
    trace!("Cutting \"{}\" with intervals", in_path.as_ref().display());

    let mut cutlist = Cutlist::try_from_intervals(intervals.as_ref())?;

    if let Err(err) = cut_with_mkvmerge(&in_path, &out_path, &cutlist) {
        return Err(CutError::Any(
            err.context(format!("Could not cut video with {}", intervals)),
        ));
    }

    // Submit cut list to cutlist.at (if that is wanted)
    if submit_cutlists {
        return match cutlist_at_access_token {
            // Access token for cutlist.at is required
            Some(access_token) => {
                if let Err(err) = cutlist.submit(in_path, tmp_dir, access_token, rating) {
                    Err(CutError::CutlistSubmissionFailed(err))
                } else {
                    Ok(())
                }
            }
            None => Err(CutError::CutlistSubmissionFailed(anyhow!(
                "No access token for cutlist.at maintained in configuration file"
            ))),
        };
    }

    Ok(())
}

/// Cut a video with a cut list retrieved from a provider by cut list id. in_path
/// is the path of the decoded video file. out_path is the path of the cut video
/// file.
fn cut_with_cutlist_from_provider_by_id<P, Q>(
    in_path: P,
    out_path: Q,
    id: u64,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    trace!(
        "Cutting \"{}\" with cut list id {} from provider",
        in_path.as_ref().display(),
        id
    );

    // Retrieve cut lists from provider and cut video
    match Cutlist::try_from(id) {
        Ok(cutlist) => match cut_with_mkvmerge(&in_path, &out_path, &cutlist) {
            Ok(_) => Ok(()),
            Err(err) => Err(CutError::Any(
                anyhow!(err).context(format!("Could not cut video with cut list {}", id)),
            )),
        },
        Err(err) => Err(CutError::Any(
            anyhow!(err).context(format!("Could not retrieve cut list ID={}", id)),
        )),
    }
}

/// Cut a video with a cut list retrieved from a provider by video file name and
/// selected automatically.
/// in_path is the path of the decoded video file.  out_path is the path of the
/// cut video file. min_cutlist_rating specifies the minimum rating a cutlist
/// must have to be accepted
fn cut_with_cutlist_from_provider_auto_select<P, Q>(
    in_path: P,
    out_path: Q,
    min_cutlist_rating: Option<CutlistRating>,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let file_name = in_path.as_ref().file_name().unwrap().to_str().unwrap();

    // Retrieve cut list headers from provider
    let headers: Vec<cutlist::ProviderHeader> =
        match cutlist::headers_from_provider(file_name, min_cutlist_rating)
            .context("Could not retrieve cut lists")
        {
            Ok(hdrs) => hdrs,
            _ => return Err(CutError::NoCutlist),
        };

    // Retrieve cut lists from provider and cut video
    let mut is_cut = false;
    for header in headers {
        match Cutlist::try_from(header.id()) {
            Ok(cutlist) => match cut_with_mkvmerge(&in_path, &out_path, &cutlist) {
                Ok(_) => {
                    is_cut = true;
                    break;
                }
                Err(err) => {
                    error!(
                        "{:?}",
                        anyhow!(err)
                            .context(format!("Could not cut video with cut list {}", header.id()))
                    );
                }
            },
            Err(err) => {
                error!(
                    "{:?}",
                    anyhow!(err).context(format!("Could not retrieve cut list {}", header.id(),))
                );
            }
        }
    }

    if !is_cut {
        return Err(CutError::Any(anyhow!(
            "No cut list could be successfully applied to cut video"
        )));
    }

    Ok(())
}

/// Cut a video file stored in in_path with mkvmerge using the given cut list.
/// The cut video is stored in out_path.
fn cut_with_mkvmerge<P, Q>(in_path: P, out_path: Q, cutlist: &Cutlist) -> anyhow::Result<()>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    fn exec_mkvmerge<P, Q>(
        in_path: P,
        out_path: Q,
        kind: &Kind,
        cutlist: &Cutlist,
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
