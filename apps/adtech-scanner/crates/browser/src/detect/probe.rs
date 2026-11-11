//! File-system and process probes behind browser detection.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::candidates::{
    Os, SearchRoots, macos_bundle_info, newest_version_dir, parse_version_line,
};

const VERSION_TIMEOUT: Duration = Duration::from_secs(4);

pub(super) fn search_roots_from_env() -> SearchRoots {
    let dir = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(Into::into)
    };
    SearchRoots {
        home: dir("HOME").or_else(|| dir("USERPROFILE")),
        path_dirs: std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default(),
        install_roots: ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"]
            .into_iter()
            .filter_map(dir)
            .collect(),
    }
}

/// What the browser calls itself and its version number, when either can be read.
pub(super) struct Identity {
    pub name: Option<String>,
    pub version: Option<String>,
}

pub(super) fn read_identity(os: Os, executable: &Path) -> Identity {
    match os {
        // Running the binary inside an app bundle can open a window (Arc does), so
        // the version comes from the bundle's Info.plist instead.
        Os::Mac => match macos_bundle_info(executable) {
            Some(info) => Identity {
                name: None,
                version: run_for_output(
                    Command::new("/usr/bin/defaults")
                        .arg("read")
                        .arg(info)
                        .arg("CFBundleShortVersionString"),
                )
                .map(|output| output.trim().to_string())
                .filter(|version| !version.is_empty()),
            },
            None => identity_from_version_flag(executable),
        },
        Os::Linux => identity_from_version_flag(executable),
        Os::Windows => Identity {
            name: None,
            version: executable.parent().and_then(version_from_install_dir),
        },
    }
}

fn identity_from_version_flag(executable: &Path) -> Identity {
    let line = run_for_output(Command::new(executable).arg("--version"))
        .as_deref()
        .and_then(parse_version_line);
    match line {
        Some(line) => Identity {
            name: Some(line.name).filter(|name| !name.is_empty()),
            version: Some(line.number),
        },
        None => Identity {
            name: None,
            version: None,
        },
    }
}

fn version_from_install_dir(dir: &Path) -> Option<String> {
    let names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    newest_version_dir(names.iter().map(String::as_str))
}

/// Runs a short command and returns its stdout, killing it if it overstays.
fn run_for_output(command: &mut Command) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    Some(output)
}
