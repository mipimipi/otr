mod cutlist;
mod ffmpeg;
mod info;
mod interval;

use anyhow::{anyhow, Context};
use log::*;
use std::{
    error::Error,
    fmt::{self, Debug, Display},
    path::Path,
    str,
};

use super::dirs::tmp_dir;
use cutlist::Cutlist;
use info::Metadata;

pub use cutlist::{
    AccessType as CutlistAccessType, Ctrl as CutlistCtrl, Rating as CutlistRating, ID as CutlistID,
};
pub use ffmpeg::is_installed as ffmpeg_is_installed;

/// Special error type for cutting videos to be able to handle specific
/// situations - e.g., if no cut list exists
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
            CutError::NoCutlist => write!(f, "No cut list exists"),
            CutError::CutlistSubmissionFailed(ref source) => {
                write!(f, "Submission of cut list to cutlist.at failed: {}", source)
            }
        }
    }
}
/// Support conversion of Error into CutError
impl Error for CutError {}
/// Support conversion of anyhow::Error into CutError
impl From<anyhow::Error> for CutError {
    fn from(err: anyhow::Error) -> CutError {
        CutError::Any(err)
    }
}

/// Cut a decoded video file.
/// - in_video is the path of the decoded video file. out_video is the path of the
///   to-be-cut video file
/// - out_video is the path of resulting file
/// - tmp_dir is the directory where OTR stores the cut list (provided a cut list
///   file is genererated and uploaded to cutlist.at) and other temporary data
/// - cutlist_ctrl contains attributes to control handling of cut lists, such as
///   - access_type: specifies how to (try to) get an appropriate cut list
///   - min_rating: specifies the minimum rating a cut list must have when
///     automatically selected from the cut list provider
///   - submit: whether cut list shall shall be uploaded to cutlist.at. In this
///     case an access token is required. Submitting cut lists does only make
///     sense if a video is cut based on intervals
///   - rating: rating of the to-be-uploaded cut lists (overwriting the default
///     which is defined in the configuration file)
pub fn cut<P, Q>(in_video: P, out_video: Q, cutlist_ctrl: &CutlistCtrl) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
{
    let tmp_dir = tmp_dir()?;

    // Call specialized cut functions based on the cut list access type that was
    // submitted
    match cutlist_ctrl.access_type {
        cutlist::AccessType::Direct(intervals) => cut_with_cutlist_from_intervals(
            in_video,
            out_video,
            tmp_dir,
            intervals,
            cutlist_ctrl.submit,
            cutlist_ctrl.access_token,
            cutlist_ctrl.rating,
        ),
        cutlist::AccessType::File(file) => {
            cut_with_cutlist_from_file(in_video, out_video, tmp_dir, file)
        }
        cutlist::AccessType::ID(id) => {
            cut_with_cutlist_from_provider_by_id(in_video, out_video, tmp_dir, id)
        }
        _ => cut_with_cutlist_from_provider_auto_select(
            in_video,
            out_video,
            tmp_dir,
            cutlist_ctrl.min_rating,
        ),
    }
}

/// Cut a video with a cut list read from an INI file. in_video is the path of the
/// decoded video file. out_video is the path of the cut video file.
fn cut_with_cutlist_from_file<P, Q, R, T>(
    in_video: P,
    out_video: Q,
    tmp_dir: T,
    cutlist_path: R,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    R: AsRef<Path>,
    T: AsRef<Path>,
{
    trace!(
        "Cutting \"{}\" with cut list from \"{}\"",
        in_video.as_ref().display(),
        cutlist_path.as_ref().display()
    );

    let cutlist = Cutlist::try_from(cutlist_path.as_ref())?;

    match cut_with_cutlist(in_video, out_video, tmp_dir, &cutlist).context(format!(
        "Could not cut video with cut list from \"{}\"",
        cutlist_path.as_ref().display()
    )) {
        Err(err) => Err(CutError::Any(err)),
        _ => Ok(()),
    }
}

/// Cut a video with a cut list derived from an intervals string. in_video is the
/// path of the decoded video file. out_video is the path of the cut video file.
/// submit_cutlists defines whether cut lists are submitted to cutlist.at. In
/// this case an access token is required
fn cut_with_cutlist_from_intervals<P, Q, T, I>(
    in_video: P,
    out_video: Q,
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
    trace!("Cutting \"{}\" with intervals", in_video.as_ref().display());

    let mut cutlist = Cutlist::try_from_intervals(intervals.as_ref())?;

    if let Err(err) = cut_with_cutlist(
        in_video.as_ref(),
        out_video.as_ref(),
        tmp_dir.as_ref(),
        &cutlist,
    ) {
        return Err(CutError::Any(
            err.context(format!("Could not cut video with {}", intervals)),
        ));
    }

    // Submit cut list to cutlist.at (if that is wanted)
    if submit_cutlists {
        return match cutlist_at_access_token {
            // Access token for cutlist.at is required
            Some(access_token) => {
                if let Err(err) = cutlist.submit(in_video, tmp_dir, access_token, rating) {
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

/// Cut a video with a cut list retrieved from a provider by cut list id. in_video
/// is the path of the decoded video file. out_video is the path of the cut video
/// file.
fn cut_with_cutlist_from_provider_by_id<P, Q, T>(
    in_video: P,
    out_video: Q,
    tmp_dir: T,
    id: u64,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    T: AsRef<Path>,
{
    trace!(
        "Cutting \"{}\" with cut list id {} from provider",
        in_video.as_ref().display(),
        id
    );

    // Retrieve cut lists from provider and cut video
    match Cutlist::try_from(id) {
        Ok(cutlist) => match cut_with_cutlist(in_video, out_video, tmp_dir, &cutlist) {
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
/// in_video is the path of the decoded video file.  out_video is the path of the
/// cut video file. min_cutlist_rating specifies the minimum rating a cut list
/// must have to be accepted
fn cut_with_cutlist_from_provider_auto_select<P, Q, T>(
    in_video: P,
    out_video: Q,
    tmp_dir: T,
    min_cutlist_rating: Option<CutlistRating>,
) -> Result<(), CutError>
where
    P: AsRef<Path>,
    Q: AsRef<Path>,
    T: AsRef<Path>,
{
    let file_name = in_video.as_ref().file_name().unwrap().to_str().unwrap();

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
            Ok(cutlist) => {
                match cut_with_cutlist(
                    in_video.as_ref(),
                    out_video.as_ref(),
                    tmp_dir.as_ref(),
                    &cutlist,
                ) {
                    Ok(_) => {
                        is_cut = true;
                        break;
                    }
                    Err(err) => {
                        error!(
                            "{:?}",
                            anyhow!(err).context(format!(
                                "Could not cut video with cut list ID={}",
                                header.id()
                            ))
                        );
                    }
                }
            }
            Err(err) => {
                error!(
                    "{:?}",
                    anyhow!(err)
                        .context(format!("Could not retrieve cut list ID={}", header.id(),))
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

/// Cut a video file stored in in_video with ffmpeg using the given cut list.
/// The cut video is stored in out_video.
fn cut_with_cutlist<I, O, T>(
    in_video: I,
    out_video: O,
    tmp_dir: T,
    cutlist: &Cutlist,
) -> anyhow::Result<()>
where
    I: AsRef<Path>,
    O: AsRef<Path>,
    T: AsRef<Path>,
{
    trace!("Cutting video with ffmpeg ...");

    // Retrieve metadata of video to be cut
    let metadata = Metadata::new(&in_video)?;

    // Try all available kinds (frame numbers, time). After the cutting was
    // successful for one of them, exit:

    // (1) Try cutting with frame intervals
    if cutlist.has_frame_intervals() {
        if !metadata.has_frames() {
            trace!("Since video has no frames, frame-based cut intervals cannot be used");
        } else if let Err(err) = ffmpeg::cut(
            &in_video,
            &out_video,
            &tmp_dir,
            cutlist.frame_intervals()?,
            &metadata,
        ) {
            warn!(
                "Could not cut \"{}\" with frame intervals: {:?}",
                in_video.as_ref().display(),
                err
            );
        } else {
            trace!("Cut video with ffmpeg");

            return Ok(());
        }
    }

    // (2) Try cutting with time intervals
    if cutlist.has_time_intervals() {
        if let Err(err) = ffmpeg::cut(
            &in_video,
            &out_video,
            &tmp_dir,
            cutlist.time_intervals()?,
            &metadata,
        ) {
            warn!(
                "Could not cut \"{}\" with time intervals: {:?}",
                in_video.as_ref().display(),
                err
            );
        } else {
            trace!("Cut video with ffmpeg");
            return Ok(());
        }
    }

    Err(anyhow!("Could not cut video with ffmpeg"))
}
