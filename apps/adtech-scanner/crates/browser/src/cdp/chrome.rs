//! The Chrome process of one capture: launched on a throwaway profile and
//! guaranteed to be gone, with its profile, when the capture is over. That
//! holds when the capture ends normally and when this process is killed:
//!
//! - Chrome is driven over `--remote-debugging-pipe` and exits by itself when
//!   the pipe closes, which the operating system does when this process ends.
//! - A watchdog kills a browser that did not exit and removes the profile.
//! - Profiles that still got left behind are removed by the next launch.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::process::Stdio;
use std::sync::Once;
use std::time::Duration;

use nix::fcntl::{FcntlArg, fcntl};
#[cfg(target_os = "linux")]
use nix::sys::signal::kill;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::unix::pipe;
use tokio::process::{Child, ChildStderr, Command};
use tokio::task::JoinHandle;

use super::profile::{self, Profile};
use super::watchdog::Watchdog;

/// Where Chrome expects the two ends of `--remote-debugging-pipe`.
const COMMANDS_FD: i32 = 3;
const REPLIES_FD: i32 = 4;
pub(super) const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_OUTPUT_LINES: usize = 20;
const STARTUP_OUTPUT_PATIENCE: Duration = Duration::from_secs(1);

static ABANDONED_PROFILES_REMOVED: Once = Once::new();

#[derive(Debug, thiserror::Error)]
pub(crate) enum LaunchError {
    #[error("could not create a temporary browser profile: {0}")]
    Profile(std::io::Error),
    #[error("could not open a connection for the browser: {0}")]
    Pipe(std::io::Error),
    #[error("could not start {path}: {source}")]
    Spawn {
        path: String,
        source: std::io::Error,
    },
    #[error("the browser exited during startup. {0}")]
    Exited(String),
    #[error("the browser did not accept commands: {0}")]
    Unresponsive(String),
    #[error("the browser did not become ready within {} seconds", STARTUP_TIMEOUT.as_secs())]
    Timeout,
}

pub(crate) struct LaunchOptions<'a> {
    pub executable: &'a Path,
    pub headless: bool,
    pub window_size: (u32, u32),
}

/// This process's ends of the browser's DevTools pipe.
pub(crate) struct DebuggingPipe {
    pub to_browser: pipe::Sender,
    pub from_browser: pipe::Receiver,
}

pub(crate) struct Chrome {
    child: Child,
    profile: Option<Profile>,
    watchdog: Option<Watchdog>,
    debugging_pipe: Option<DebuggingPipe>,
    startup_output: Option<JoinHandle<Vec<String>>>,
}

impl Chrome {
    /// Starts the process. From here on, dropping the value kills Chrome and
    /// deletes the profile, whatever state startup is in.
    pub(crate) fn spawn(options: LaunchOptions<'_>) -> Result<Self, LaunchError> {
        let profiles = std::env::temp_dir();
        ABANDONED_PROFILES_REMOVED.call_once(|| {
            let profiles = profiles.clone();
            tokio::task::spawn_blocking(move || profile::remove_abandoned(&profiles));
        });
        let profile = Profile::create_in(&profiles).map_err(LaunchError::Profile)?;
        let (commands_for_browser, to_browser) = std::io::pipe().map_err(LaunchError::Pipe)?;
        let (from_browser, replies_from_browser) = std::io::pipe().map_err(LaunchError::Pipe)?;
        let debugging_pipe = DebuggingPipe {
            to_browser: pipe::Sender::from_owned_fd(to_browser.into())
                .map_err(LaunchError::Pipe)?,
            from_browser: pipe::Receiver::from_owned_fd(from_browser.into())
                .map_err(LaunchError::Pipe)?,
        };
        let commands_for_browser =
            above_the_browser_fds(commands_for_browser.into()).map_err(LaunchError::Pipe)?;
        let replies_from_browser =
            above_the_browser_fds(replies_from_browser.into()).map_err(LaunchError::Pipe)?;

        let mut command = Command::new(options.executable);
        command
            .args(launch_arguments(&options, profile.path()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            // Its own process group, so renderers and helpers can be killed with it.
            .process_group(0);
        // SAFETY: the closure runs in the forked child before exec and only
        // calls `dup2`, which is async-signal-safe. Both sources are open
        // descriptors above the targets, owned by the closure.
        unsafe {
            command.pre_exec(move || {
                for (source, target) in [
                    (commands_for_browser.as_raw_fd(), COMMANDS_FD),
                    (replies_from_browser.as_raw_fd(), REPLIES_FD),
                ] {
                    if nix::libc::dup2(source, target) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|source| LaunchError::Spawn {
            path: options.executable.display().to_string(),
            source,
        })?;
        // Closes this process's copies of the browser's pipe ends. While they
        // are open, neither side would ever see the other one go away.
        drop(command);

        // Best effort: a launch does not fail for want of its second line of
        // defence against being killed.
        let watchdog = child
            .id()
            .and_then(|group| Watchdog::guard(group, profile.path()).ok());
        let startup_output = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(drain_stderr(stderr)));
        Ok(Self {
            child,
            profile: Some(profile),
            watchdog,
            debugging_pipe: Some(debugging_pipe),
            startup_output,
        })
    }

    /// The connection to the browser. There is one; later calls get `None`.
    pub(crate) fn take_debugging_pipe(&mut self) -> Option<DebuggingPipe> {
        self.debugging_pipe.take()
    }

    /// What Chrome printed while starting, for when it turns out to have
    /// died. Waits a moment for the output to be complete.
    pub(crate) async fn startup_output(&mut self) -> String {
        let Some(output) = self.startup_output.take() else {
            return String::new();
        };
        match tokio::time::timeout(STARTUP_OUTPUT_PATIENCE, output).await {
            Ok(Ok(lines)) => lines.join(" | "),
            _ => String::new(),
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
            let helpers_left = kill_helpers(&path);
            let group_left = group.is_some_and(group_is_running);
            let profile_gone = !path.exists() || std::fs::remove_dir_all(&path).is_ok();
            if profile_gone && !helpers_left && !group_left {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn kill_process_tree(&mut self) {
        if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
        let _ = self.child.start_kill();
        if let Some(profile) = &self.profile {
            kill_helpers(profile.path());
        }
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        self.kill_process_tree();
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.disarm();
        }
    }
}

/// Moves a descriptor out of the range Chrome's pipe ends are placed in, so
/// that placing one of them cannot overwrite the other. A process with few
/// open files gets its new descriptors numbered from 3.
fn above_the_browser_fds(fd: OwnedFd) -> std::io::Result<OwnedFd> {
    let moved = fcntl(&fd, FcntlArg::F_DUPFD_CLOEXEC(REPLIES_FD + 1))?;
    // SAFETY: `fcntl` just returned this descriptor; nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(moved) })
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

#[cfg(not(target_os = "linux"))]
fn group_is_running(group: u32) -> bool {
    i32::try_from(group).is_ok_and(|group| killpg(Pid::from_raw(group), None).is_ok())
}

fn launch_arguments(options: &LaunchOptions<'_>, profile: &Path) -> Vec<String> {
    let (width, height) = options.window_size;
    let mut arguments = vec![
        format!("--user-data-dir={}", profile.display()),
        "--remote-debugging-pipe".to_string(),
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

fn running_as_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

/// Keeps the first lines Chrome prints, then goes on reading so that Chrome
/// never blocks on a full pipe. Ends when Chrome and its helpers are gone.
async fn drain_stderr(stderr: ChildStderr) -> Vec<String> {
    let mut startup_output = Vec::new();
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if startup_output.len() < STARTUP_OUTPUT_LINES {
            startup_output.push(line);
        }
    }
    startup_output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_browser_that_dies_while_starting_is_reported_with_what_it_printed() {
        // A shell rejects Chrome's arguments, complains and exits.
        let mut chrome = Chrome::spawn(LaunchOptions {
            executable: Path::new("/bin/sh"),
            headless: true,
            window_size: (800, 600),
        })
        .unwrap();

        let Err(error) = crate::cdp::connect(&mut chrome).await else {
            panic!("a shell is not a browser");
        };

        let LaunchError::Exited(output) = &error else {
            panic!("{error:?}");
        };
        assert!(output.contains("/bin/sh"), "{output}");
        chrome.shutdown().await;
    }

    /// The path a panicking or abandoned capture takes: no `shutdown`.
    #[cfg(target_os = "linux")]
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
        crate::cdp::connect(&mut chrome).await.unwrap();
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
