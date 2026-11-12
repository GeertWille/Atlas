//! The Chrome process of one capture: launched on a throwaway profile and
//! guaranteed to be gone, with its profile, when the capture is over.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

#[cfg(target_os = "linux")]
use nix::sys::signal::kill;
#[cfg(unix)]
use nix::sys::signal::{Signal, killpg};
#[cfg(unix)]
use nix::unistd::Pid;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, ChildStderr, Command};
use tokio::sync::oneshot;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const DEVTOOLS_BANNER: &str = "DevTools listening on ";

#[derive(Debug, thiserror::Error)]
pub(crate) enum LaunchError {
    #[error("could not create a temporary browser profile: {0}")]
    Profile(std::io::Error),
    #[error("could not start {path}: {source}")]
    Spawn {
        path: String,
        source: std::io::Error,
    },
    #[error("the browser exited during startup. {0}")]
    Exited(String),
    #[error("the browser did not become ready within {} seconds", STARTUP_TIMEOUT.as_secs())]
    Timeout,
}

pub(crate) struct LaunchOptions<'a> {
    pub executable: &'a Path,
    pub headless: bool,
    pub window_size: (u32, u32),
}

pub(crate) struct Chrome {
    child: Child,
    profile: Option<TempDir>,
    devtools_url: Option<oneshot::Receiver<Result<String, String>>>,
}

impl Chrome {
    /// Starts the process. From here on, dropping the value kills Chrome and
    /// deletes the profile, whatever state startup is in.
    pub(crate) fn spawn(options: LaunchOptions<'_>) -> Result<Self, LaunchError> {
        let profile = tempfile::Builder::new()
            .prefix("adscan-profile-")
            .tempdir()
            .map_err(LaunchError::Profile)?;
        let mut command = Command::new(options.executable);
        command
            .args(launch_arguments(&options, profile.path()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Its own process group, so renderers and helpers can be killed with it.
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|source| LaunchError::Spawn {
            path: options.executable.display().to_string(),
            source,
        })?;
        let (found, devtools_url) = oneshot::channel();
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(watch_stderr(stderr, found));
        }
        Ok(Self {
            child,
            profile: Some(profile),
            devtools_url: Some(devtools_url),
        })
    }

    /// Waits for Chrome to announce its DevTools WebSocket endpoint.
    pub(crate) async fn devtools_url(&mut self) -> Result<String, LaunchError> {
        let announced = self.devtools_url.take().ok_or(LaunchError::Timeout)?;
        match tokio::time::timeout(STARTUP_TIMEOUT, announced).await {
            Ok(Ok(Ok(url))) => Ok(url),
            Ok(Ok(Err(stderr))) => Err(LaunchError::Exited(stderr)),
            Ok(Err(_)) => Err(LaunchError::Exited(String::new())),
            Err(_) => Err(LaunchError::Timeout),
        }
    }

    /// Kills the whole process tree and removes the profile; both are gone
    /// when this returns.
    pub(crate) async fn shutdown(mut self) {
        let group = self.child.id();
        self.kill_process_tree();
        let _ = self.child.wait().await;
        let Some(profile) = self.profile.take() else {
            return;
        };
        let path = profile.path().to_path_buf();
        drop(profile);
        // Killed processes need a moment to go and release their files.
        for _ in 0..20 {
            let processes_left =
                kill_helpers(&path) | group.is_some_and(|group| group_is_running(group));
            let profile_gone = !path.exists() || std::fs::remove_dir_all(&path).is_ok();
            if profile_gone && !processes_left {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn kill_process_tree(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
        let _ = self.child.start_kill();
        if let Some(profile) = &self.profile {
            kill_helpers(profile.path());
        }
    }
}

/// Kills every live process started with this profile and returns whether
/// there was one. Chrome's crash handler detaches into its own session, out
/// of reach of the process-group kill, and keeps writing into the profile
/// while it winds down; like every Chrome process, it carries the profile
/// path on its command line.
#[cfg(target_os = "linux")]
fn kill_helpers(profile: &Path) -> bool {
    let profile = profile.to_string_lossy();
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return false;
    };
    let mut found = false;
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let uses_profile =
            std::fs::read(process.path().join("cmdline")).is_ok_and(|command_line| {
                String::from_utf8_lossy(&command_line).contains(profile.as_ref())
            });
        if uses_profile {
            let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
            found = true;
        }
    }
    found
}

#[cfg(not(target_os = "linux"))]
fn kill_helpers(_profile: &Path) -> bool {
    false
}

/// Whether a process of the killed browser's group has yet to exit. Zombies
/// do not count: nothing may be left to reap them.
#[cfg(target_os = "linux")]
fn group_is_running(group: u32) -> bool {
    let group = group.to_string();
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return false;
    };
    processes.flatten().any(|process| {
        let Ok(stat) = std::fs::read_to_string(process.path().join("stat")) else {
            return false;
        };
        // "pid (comm) state ppid pgrp ..."; comm may contain spaces.
        let mut fields = stat
            .rsplit_once(')')
            .map_or("", |(_, rest)| rest)
            .split_whitespace();
        let is_zombie = fields.next() == Some("Z");
        !is_zombie && fields.nth(1) == Some(group.as_str())
    })
}

#[cfg(all(unix, not(target_os = "linux")))]
fn group_is_running(group: u32) -> bool {
    i32::try_from(group).is_ok_and(|group| killpg(Pid::from_raw(group), None).is_ok())
}

#[cfg(not(unix))]
fn group_is_running(_group: u32) -> bool {
    false
}

impl Drop for Chrome {
    fn drop(&mut self) {
        self.kill_process_tree();
    }
}

fn launch_arguments(options: &LaunchOptions<'_>, profile: &Path) -> Vec<String> {
    let (width, height) = options.window_size;
    let mut arguments = vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-port=0".to_string(),
        format!("--window-size={width},{height}"),
    ];
    arguments.extend(
        [
            "--no-first-run",
            "--no-default-browser-check",
            // Fresh profiles in the EEA otherwise open on a search engine dialog.
            "--disable-search-engine-choice-screen",
            "--disable-default-apps",
            // The scan's only traffic should be the scanned page's. Chrome's own
            // services (account listing, search suggestions) cannot all be switched
            // off, so the ones that remain are pointed at a closed local port.
            "--disable-background-networking",
            "--disable-client-side-phishing-detection",
            "--google-base-url=http://127.0.0.1:9/",
            "--gaia-url=http://127.0.0.1:9/",
            "--disable-extensions",
            "--disable-component-update",
            "--disable-sync",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            // Without this `navigator.webdriver` is true and ad stacks see a bot.
            "--disable-blink-features=AutomationControlled",
            "--disable-features=Translate,MediaRouter,OptimizationHints,DialMediaRouteProvider,NetworkTimeServiceQuerying",
            "--password-store=basic",
            "--use-mock-keychain",
            "--mute-audio",
        ]
        .map(str::to_string),
    );
    if options.headless {
        arguments.push("--headless=new".to_string());
        // Headless Chrome has no mouse, so a desktop page would see
        // `(hover: none)` and no fine pointer, like a touch device. Phone
        // emulation replaces these with touch values.
        arguments.push(
            "--blink-settings=primaryHoverType=2,availableHoverTypes=2,\
             primaryPointerType=4,availablePointerTypes=4"
                .to_string(),
        );
    }
    if cfg!(target_os = "linux") {
        // Containers ship a /dev/shm too small for Chrome's shared memory.
        arguments.push("--disable-dev-shm-usage".to_string());
        if running_as_root() {
            arguments.push("--no-sandbox".to_string());
        }
    }
    arguments.push("about:blank".to_string());
    arguments
}

#[cfg(unix)]
fn running_as_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

#[cfg(not(unix))]
fn running_as_root() -> bool {
    false
}

/// Reports the DevTools endpoint (or what Chrome said before dying), then keeps
/// draining stderr so Chrome never blocks on a full pipe.
async fn watch_stderr(stderr: ChildStderr, found: oneshot::Sender<Result<String, String>>) {
    let mut found = Some(found);
    let mut startup_output = Vec::new();
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if found.is_none() {
            continue;
        }
        if let Some(url) = line.trim().strip_prefix(DEVTOOLS_BANNER) {
            if let Some(found) = found.take() {
                let _ = found.send(Ok(url.to_string()));
            }
        } else if startup_output.len() < 20 {
            startup_output.push(line);
        }
    }
    if let Some(found) = found {
        let _ = found.send(Err(startup_output.join(" | ")));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// The path a panicking or abandoned capture takes: no `shutdown`.
    #[tokio::test]
    async fn dropping_a_running_browser_removes_its_profile() {
        let Some(browser) = crate::detect::detect_browser(None) else {
            return;
        };
        let mut chrome = Chrome::spawn(LaunchOptions {
            executable: Path::new(&browser.path),
            headless: true,
            window_size: (800, 600),
        })
        .unwrap();
        chrome.devtools_url().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let profile = chrome.profile.as_ref().unwrap().path().to_path_buf();
        assert!(profile.join("Default").exists());

        drop(chrome);

        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while profile.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!profile.exists(), "{} was left behind", profile.display());
    }
}
