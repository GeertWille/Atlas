//! Where Chromium-family browsers live on each operating system, and how to read
//! their identity from a path or a `--version` line. No I/O.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Os {
    MacOs,
    Linux,
    Windows,
}

impl Os {
    pub(crate) fn current() -> Self {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// The parts of the process environment that decide where to look.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct SearchRoots {
    pub home: Option<PathBuf>,
    /// Directories of `PATH`, in order.
    pub path_dirs: Vec<PathBuf>,
    /// Windows install roots (`ProgramFiles`, `ProgramFiles(x86)`, `LocalAppData`).
    pub install_roots: Vec<PathBuf>,
}

const MAC_APPS: &[(&str, &str)] = &[
    ("Google Chrome.app", "Google Chrome"),
    ("Google Chrome Beta.app", "Google Chrome Beta"),
    ("Chromium.app", "Chromium"),
    ("Brave Browser.app", "Brave Browser"),
    ("Microsoft Edge.app", "Microsoft Edge"),
    ("Google Chrome Canary.app", "Google Chrome Canary"),
    ("Arc.app", "Arc"),
];

const LINUX_COMMANDS: &[&str] = &[
    "google-chrome-stable",
    "google-chrome",
    "google-chrome-beta",
    "chromium",
    "chromium-browser",
    "brave-browser",
    "microsoft-edge-stable",
    "microsoft-edge",
];

const LINUX_PATHS: &[&str] = &[
    "/opt/google/chrome/chrome",
    "/opt/google/chrome-beta/chrome",
    "/usr/lib/chromium/chromium",
    "/usr/lib/chromium-browser/chromium-browser",
    "/snap/bin/chromium",
    "/var/lib/flatpak/exports/bin/com.google.Chrome",
    "/var/lib/flatpak/exports/bin/org.chromium.Chromium",
    "/opt/brave.com/brave/brave",
    "/opt/microsoft/msedge/msedge",
];

const WINDOWS_INSTALLS: &[&[&str]] = &[
    &["Google", "Chrome", "Application", "chrome.exe"],
    &["Google", "Chrome Beta", "Application", "chrome.exe"],
    &["Chromium", "Application", "chrome.exe"],
    &["BraveSoftware", "Brave-Browser", "Application", "brave.exe"],
    &["Microsoft", "Edge", "Application", "msedge.exe"],
    &["Google", "Chrome SxS", "Application", "chrome.exe"],
];

/// Executable paths worth checking, most preferred first.
pub(crate) fn well_known_paths(os: Os, roots: &SearchRoots) -> Vec<PathBuf> {
    match os {
        Os::MacOs => {
            let mut bases = vec![PathBuf::from("/Applications")];
            bases.extend(roots.home.iter().map(|home| home.join("Applications")));
            MAC_APPS
                .iter()
                .flat_map(|(bundle, binary)| {
                    bases
                        .iter()
                        .map(move |base| base.join(bundle).join("Contents/MacOS").join(binary))
                })
                .collect()
        }
        Os::Linux => LINUX_COMMANDS
            .iter()
            .flat_map(|command| roots.path_dirs.iter().map(move |dir| dir.join(command)))
            .chain(LINUX_PATHS.iter().map(PathBuf::from))
            .collect(),
        Os::Windows => WINDOWS_INSTALLS
            .iter()
            .flat_map(|parts| {
                roots
                    .install_roots
                    .iter()
                    .map(move |root| parts.iter().fold(root.clone(), |path, part| path.join(part)))
            })
            .collect(),
    }
}

/// Browser family guessed from the executable path alone.
pub(crate) fn family_name(path: &Path) -> &'static str {
    let lower = path.to_string_lossy().to_lowercase();
    let has = |needle: &str| lower.contains(needle);
    if has("brave") {
        "Brave Browser"
    } else if has("msedge") || has("microsoft edge") || has("microsoft-edge") {
        "Microsoft Edge"
    } else if has("arc.app") {
        "Arc"
    } else if has("canary") || has("chrome sxs") {
        "Google Chrome Canary"
    } else if has("chrome beta") || has("chrome-beta") {
        "Google Chrome Beta"
    } else if has("chromium") {
        "Chromium"
    } else if has("chrome") {
        "Google Chrome"
    } else {
        "Chromium-based browser"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionLine {
    pub name: String,
    pub number: String,
}

/// Parses `--version` output such as `Google Chrome 141.0.7390.55 beta`.
pub(crate) fn parse_version_line(output: &str) -> Option<VersionLine> {
    let line = output.lines().find(|line| !line.trim().is_empty())?;
    let words: Vec<&str> = line.split_whitespace().collect();
    let position = words.iter().position(|word| is_version_number(word))?;
    Some(VersionLine {
        name: words[..position].join(" "),
        number: words[position].to_string(),
    })
}

fn is_version_number(word: &str) -> bool {
    word.contains('.')
        && word.starts_with(|c: char| c.is_ascii_digit())
        && word.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// `Info` plist domain of the macOS app bundle that contains `executable`, as
/// accepted by `defaults read`. Reading it never launches the app.
pub(crate) fn macos_bundle_info(executable: &Path) -> Option<PathBuf> {
    let macos_dir = executable.parent()?;
    let contents = macos_dir.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos_dir.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| contents.join("Info"))
}

/// Chrome on Windows prints nothing for `--version`, but keeps a directory
/// named after the installed version next to the executable.
pub(crate) fn newest_version_dir<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    names
        .into_iter()
        .filter(|name| is_version_number(name))
        .max_by_key(|name| {
            name.split('.')
                .map(|part| part.parse::<u64>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> SearchRoots {
        SearchRoots {
            home: Some(PathBuf::from("/Users/ada")),
            path_dirs: vec![PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")],
            install_roots: vec![PathBuf::from("C:/Program Files"), PathBuf::from("C:/Users/ada/AppData/Local")],
        }
    }

    #[test]
    fn macos_prefers_chrome_and_checks_both_application_folders() {
        let paths = well_known_paths(Os::MacOs, &roots());
        assert_eq!(
            paths[0],
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        );
        assert_eq!(
            paths[1],
            PathBuf::from("/Users/ada/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        );
        assert_eq!(paths.len(), MAC_APPS.len() * 2);
        for app in ["Chrome Beta", "Chrome Canary", "Chromium", "Brave Browser", "Microsoft Edge", "Arc"] {
            assert!(
                paths.iter().any(|path| path.to_string_lossy().ends_with(app)),
                "missing {app}"
            );
        }
        assert!(paths.last().unwrap().ends_with("Arc.app/Contents/MacOS/Arc"));
    }

    #[test]
    fn macos_without_home_only_checks_system_applications() {
        let paths = well_known_paths(Os::MacOs, &SearchRoots::default());
        assert_eq!(paths.len(), MAC_APPS.len());
        assert!(paths.iter().all(|path| path.starts_with("/Applications")));
    }

    #[test]
    fn linux_checks_path_commands_before_fixed_locations() {
        let paths = well_known_paths(Os::Linux, &roots());
        assert_eq!(paths[0], PathBuf::from("/usr/local/bin/google-chrome-stable"));
        assert_eq!(paths[1], PathBuf::from("/usr/bin/google-chrome-stable"));
        let first_fixed = paths
            .iter()
            .position(|path| path == Path::new("/opt/google/chrome/chrome"))
            .unwrap();
        assert_eq!(first_fixed, LINUX_COMMANDS.len() * 2);
        assert!(paths.contains(&PathBuf::from("/usr/bin/chromium-browser")));
    }

    #[test]
    fn windows_combines_every_install_root() {
        let paths = well_known_paths(Os::Windows, &roots());
        assert_eq!(
            paths[0],
            PathBuf::from("C:/Program Files/Google/Chrome/Application/chrome.exe")
        );
        assert!(paths.contains(&PathBuf::from(
            "C:/Users/ada/AppData/Local/Microsoft/Edge/Application/msedge.exe"
        )));
        assert_eq!(paths.len(), WINDOWS_INSTALLS.len() * 2);
    }

    #[test]
    fn family_is_read_from_the_path() {
        let cases = [
            ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "Google Chrome"),
            ("/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta", "Google Chrome Beta"),
            ("/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary", "Google Chrome Canary"),
            ("/usr/bin/chromium-browser", "Chromium"),
            ("/opt/brave.com/brave/brave", "Brave Browser"),
            ("C:/Program Files/Microsoft/Edge/Application/msedge.exe", "Microsoft Edge"),
            ("/Applications/Arc.app/Contents/MacOS/Arc", "Arc"),
            ("/opt/pw-browsers/chromium-1243/chrome-linux64/chrome", "Chromium"),
            ("/opt/thing/browser", "Chromium-based browser"),
        ];
        for (path, expected) in cases {
            assert_eq!(family_name(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn version_line_splits_name_and_number() {
        let parsed = parse_version_line("Google Chrome for Testing 153.0.8010.12 \n").unwrap();
        assert_eq!(parsed.name, "Google Chrome for Testing");
        assert_eq!(parsed.number, "153.0.8010.12");

        let parsed = parse_version_line("\nChromium 141.0.7390.55 snap\n").unwrap();
        assert_eq!((parsed.name.as_str(), parsed.number.as_str()), ("Chromium", "141.0.7390.55"));

        assert_eq!(parse_version_line("141.0.1").unwrap().name, "");
        assert_eq!(parse_version_line("command not found"), None);
        assert_eq!(parse_version_line(""), None);
    }

    #[test]
    fn macos_bundle_info_is_derived_only_for_app_bundles() {
        assert_eq!(
            macos_bundle_info(Path::new("/Applications/Arc.app/Contents/MacOS/Arc")),
            Some(PathBuf::from("/Applications/Arc.app/Contents/Info"))
        );
        assert_eq!(macos_bundle_info(Path::new("/usr/bin/chromium")), None);
        assert_eq!(macos_bundle_info(Path::new("/opt/Contents/MacOS/chrome")), None);
    }

    #[test]
    fn newest_version_dir_ignores_other_entries() {
        let names = ["chrome.exe", "140.0.7339.208", "141.0.7390.55", "SetupMetrics", "9.0.1.2"];
        assert_eq!(newest_version_dir(names).as_deref(), Some("141.0.7390.55"));
        assert_eq!(newest_version_dir(["chrome.exe"]), None);
    }
}
