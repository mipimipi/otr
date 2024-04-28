use anyhow::{anyhow, Context};
use log::*;
use once_cell::sync::OnceCell;
use std::{fs, path::PathBuf};

pub const OTR_DEFAULT_DIR: &str = "OTR";

/// Temporary/cache directory of otr. It is <CACHE_DIR_OF_YOUR_OS>/OTR. The
/// determination is only done once. The result is stored in a static variable
pub fn tmp_dir() -> anyhow::Result<&'static PathBuf> {
    static TMP_DIR: OnceCell<PathBuf> = OnceCell::new();
    TMP_DIR.get_or_try_init(|| {
        let dir = if let Some(tmp_dir) = dirs::cache_dir() {
            trace!("Cache directory determined: Assemble temp directory");
            let tmp_dir = tmp_dir.join(OTR_DEFAULT_DIR);

            // Create temp directory (if it does not exist)
            fs::create_dir_all(&tmp_dir).with_context(|| {
                format!(
                    "Could not create temp directory \"{}\"",
                    tmp_dir.as_path().display()
                )
            })?;

            tmp_dir
        } else {
            trace!("Cache directory could not be determined");
            return Err(anyhow!("Temp directory could not be determined"));
        };

        debug!("Temp directory: {:?}", dir);

        Ok(dir)
    })
}
