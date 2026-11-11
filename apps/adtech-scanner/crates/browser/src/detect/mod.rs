//! Finds the Chrome/Chromium installation to drive.

mod candidates;
mod probe;

use std::path::{Path, PathBuf};

use adscan_model::BrowserInfo;

use candidates::{Os, family_name, well_known_paths};

/// Overrides every other source; used by tests and local development.
pub(crate) const CHROME_PATH_ENV: &str = "ADSCAN_CHROME_PATH";

pub(crate) fn detect_browser(explicit: Option<&str>) -> Option<BrowserInfo> {
    let os = Os::current();
    let from_env = std::env::var_os(CHROME_PATH_ENV).map(PathBuf::from);
    let configured = explicit.filter(|path| !path.trim().is_empty()).map(PathBuf::from);
    let executable = from_env
        .into_iter()
        .chain(configured)
        .chain(well_known_paths(os, &probe::search_roots_from_env()))
        .find(|path| path.is_file())?;
    Some(describe(os, &executable))
}

fn describe(os: Os, executable: &Path) -> BrowserInfo {
    let identity = probe::read_identity(os, executable);
    let name = identity.name.unwrap_or_else(|| family_name(executable).to_string());
    let version = match identity.version {
        Some(number) => format!("{name} {number}"),
        None => name.clone(),
    };
    BrowserInfo {
        path: executable.to_string_lossy().into_owned(),
        version,
        name,
    }
}
