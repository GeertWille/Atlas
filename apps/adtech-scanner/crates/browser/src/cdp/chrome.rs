//! The Chrome process of one capture, guaranteed to be gone when the capture
//! is over. That holds when the capture ends normally and when this process
//! is killed:
//!
//! - Chrome is driven over `--remote-debugging-pipe` and exits by itself when
//!   the pipe closes, which the operating system does when this process ends.
//! - A watchdog kills a browser that did not exit.
//!
//! A capture normally runs on a throwaway profile, which goes with the
//! browser: removed at shutdown, by the watchdog, or by the next launch when
//! both were prevented. A capture on the browser profile the user keeps leaves
//! that directory alone on every one of those paths.

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

use super::profile::{self, PersistentProfile, PersistentProfileError, Profile};
use super::watchdog::Watchdog;

/// Where Chrome expects the two ends of `--remote-debugging-pipe`.
const COMMANDS_FD: i32 = 3;
const REPLIES_FD: i32 = 4;
pub(super) const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_OUTPUT_LINES: usize = 20;
const STARTUP_OUTPUT_PATIENCE: Duration = Duration::from_secs(1);
/// How often, 50 ms apart, a shutdown looks whether the killed browser is gone.
const SHUTDOWN_CHECKS: usize = 100;

static ABANDONED_PROFILES_REMOVED: Once = Once::new();

#[derive(Debug, thiserror::Error)]
pub(crate) enum LaunchError {
    #[error("could not create a temporary browser profile: {0}")]
    Profile(std::io::Error),
    #[error(transparent)]
    PersistentProfile(#[from] PersistentProfileError),
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
    pub profile: ProfileChoice<'a>,
    pub proxy: &'a ProxyServer,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProfileChoice<'a> {
    /// A fresh profile, deleted with the browser.
    Throwaway,
    /// The browser profile at this path, kept.
    Persistent(&'a Path),
}

/// How the browser reaches the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProxyServer {
    /// Whatever the operating system is configured to use: Chrome's default.
    System,
    /// No proxy, whatever the operating system says.
    Direct,
    /// `scheme` is `http`, `https` or `socks5`.
    Custom {
        scheme: String,
        host: String,
        port: u16,
    },
}

enum ProfileInUse {
    Throwaway(Profile),
    Persistent(PersistentProfile),
}

impl ProfileInUse {
    fn path(&self) -> &Path {
        match self {
            ProfileInUse::Throwaway(profile) => profile.path(),
            ProfileInUse::Persistent(profile) => profile.path(),
        }
    }

    /// The directory to delete with the browser, if it is one.
    fn disposable(&self) -> Option<&Path> {
        match self {
            ProfileInUse::Throwaway(profile) => Some(profile.path()),
            ProfileInUse::Persistent(_) => None,
        }
    }
}

/// This process's ends of the browser's DevTools pipe.
pub(crate) struct DebuggingPipe {
    pub to_browser: pipe::Sender,
    pub from_browser: pipe::Receiver,
}

pub(crate) struct Chrome {
    child: Child,
    /// The browser's process group, which is its process id. Remembered
    /// because the child forgets its id once it was waited for, and its
    /// helpers can outlive it for a moment.
    group: Option<u32>,
    profile: Option<ProfileInUse>,
    watchdog: Option<Watchdog>,
    debugging_pipe: Option<DebuggingPipe>,
    startup_output: Option<JoinHandle<Vec<String>>>,
}

impl Chrome {
    /// Starts the process. From here on, dropping the value kills Chrome and
    /// deletes a throwaway profile, whatever state startup is in.
    pub(crate) fn spawn(options: LaunchOptions<'_>) -> Result<Self, LaunchError> {
        let profiles = std::env::temp_dir();
        ABANDONED_PROFILES_REMOVED.call_once(|| {
            let profiles = profiles.clone();
            tokio::task::spawn_blocking(move || profile::remove_abandoned(&profiles));
        });
        let profile = match options.profile {
            ProfileChoice::Throwaway => ProfileInUse::Throwaway(
                Profile::create_in(&profiles).map_err(LaunchError::Profile)?,
            ),
            ProfileChoice::Persistent(path) => {
                let profile = PersistentProfile::open(path)?;
                // A browser that outlived a crashed scan would fight this one
                // over the profile's files.
                kill_helpers(path);
                ProfileInUse::Persistent(profile)
            }
        };
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
        let group = child.id();
        let watchdog = group.and_then(|group| Watchdog::guard(group, profile.disposable()).ok());
        let startup_output = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(drain_stderr(stderr)));
        Ok(Self {
            child,
            group,
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

    /// Gives a browser that was asked to close (`Browser.close`) up to
    /// `patience` to finish writing its profile and exit.
    pub(crate) async fn wait_for_exit(&mut self, patience: Duration) {
        let _ = tokio::time::timeout(patience, self.child.wait()).await;
    }

    /// Whether the profile of this launch is kept after it.
    pub(crate) fn keeps_its_profile(&self) -> bool {
        matches!(self.profile, Some(ProfileInUse::Persistent(_)))
    }

    /// Kills the whole process tree and removes a throwaway profile; both
    /// are gone when this returns.
    pub(crate) async fn shutdown(mut self) {
        let group = self.group;
        self.kill_process_tree();
        let _ = self.child.wait().await;
        let Some(profile) = self.profile.take() else {
            return;
        };
        let path = profile.path().to_path_buf();
        let disposable = profile.disposable().is_some();
        // Killed processes need a moment to go and release their files. The
        // lock on a kept profile is held until they are gone.
        for _ in 0..SHUTDOWN_CHECKS {
            let helpers_left = kill_helpers(&path);
            let group_left = group.is_some_and(group_is_running);
            let profile_done =
                !disposable || !path.exists() || std::fs::remove_dir_all(&path).is_ok();
            if profile_done && !helpers_left && !group_left {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn kill_process_tree(&mut self) {
        if let Some(group) = self.group.and_then(|group| i32::try_from(group).ok()) {
            let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
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
        let uses_profile = std::fs::read(process.path().join("cmdline"))
            .is_ok_and(|command_line| runs_on_profile(&command_line, profile));
        if uses_profile {
            let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
            found = true;
        }
    }
    found
}

/// Whether a command line (arguments separated by NUL, as `/proc` gives it)
/// has a switch whose value is the profile directory or a path inside it:
/// `--user-data-dir=<profile>`, the crash handler's
/// `--database=<profile>/Crashpad`. A process that merely mentions the path,
/// or runs on a directory whose name starts the same, is not the browser's.
#[cfg(any(target_os = "linux", test))]
fn runs_on_profile(command_line: &[u8], profile: &Path) -> bool {
    let command_line = String::from_utf8_lossy(command_line);
    command_line.split('\0').any(|argument| {
        argument
            .strip_prefix("--")
            .and_then(|switch| switch.split_once('='))
            .is_some_and(|(_, value)| Path::new(value).starts_with(profile))
    })
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

/// The browser profile's HTTP cache may grow to this size on disk.
const KEPT_PROFILE_CACHE_BYTES: u64 = 100 * 1024 * 1024;

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
            // A window that is hidden or covered must keep its timers running
            // at full speed: auction timeouts are measured with them.
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            "--disable-features=Translate,MediaRouter,OptimizationHints,DialMediaRouteProvider,NetworkTimeServiceQuerying",
            "--password-store=basic",
            "--use-mock-keychain",
            "--mute-audio",
        ]
        .map(str::to_string),
    );
    // Whoever drives it, the window is the one the user would open themselves:
    // Chrome's automation marker (`navigator.webdriver`) stays off, so pages
    // serve the scan what they serve a person. The audit trail still reports
    // what the page actually saw.
    arguments.push("--disable-blink-features=AutomationControlled".to_string());
    if matches!(options.profile, ProfileChoice::Persistent(_)) {
        arguments.push(format!("--disk-cache-size={KEPT_PROFILE_CACHE_BYTES}"));
    }
    arguments.extend(proxy_arguments(options.proxy));
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

/// Never carries credentials: a command line is readable by every process of
/// the user. A proxy's challenge is answered over the DevTools connection.
///
/// Whatever the proxy, Chrome connects to loopback addresses directly unless
/// told otherwise, so a site served from this machine is always reachable.
fn proxy_arguments(proxy: &ProxyServer) -> Vec<String> {
    match proxy {
        ProxyServer::System => Vec::new(),
        ProxyServer::Direct => vec!["--no-proxy-server".to_string()],
        ProxyServer::Custom { scheme, host, port } => {
            let host = if host.contains(':') && !host.starts_with('[') {
                format!("[{host}]")
            } else {
                host.clone()
            };
            vec![format!("--proxy-server={scheme}://{host}:{port}")]
        }
    }
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

    fn options(executable: &Path) -> LaunchOptions<'_> {
        LaunchOptions {
            executable,
            headless: true,
            window_size: (800, 600),
            profile: ProfileChoice::Throwaway,
            proxy: &ProxyServer::System,
        }
    }

    fn arguments(options: &LaunchOptions<'_>) -> Vec<String> {
        launch_arguments(options, Path::new("/tmp/profile"))
    }

    #[test]
    fn no_visit_is_flagged_as_automated_whoever_drives_it() {
        let executable = Path::new("chrome");
        for scripted in [
            arguments(&options(executable)),
            arguments(&LaunchOptions {
                headless: false,
                ..options(executable)
            }),
        ] {
            assert!(
                scripted.contains(&"--disable-blink-features=AutomationControlled".to_string())
            );
            assert!(
                !scripted
                    .iter()
                    .any(|flag| flag.contains("enable-automation"))
            );
            assert!(!scripted.iter().any(|flag| flag.contains("user-agent")));
        }

        let visible = arguments(&LaunchOptions {
            headless: false,
            ..options(executable)
        });
        assert!(!visible.iter().any(|flag| flag.starts_with("--headless")));
        assert!(
            !visible
                .iter()
                .any(|flag| flag.starts_with("--blink-settings"))
        );
    }

    #[test]
    fn the_proxy_setting_becomes_flags_without_credentials() {
        let executable = Path::new("chrome");
        let proxy_flags = |proxy: &ProxyServer| -> Vec<String> {
            arguments(&LaunchOptions {
                proxy,
                ..options(executable)
            })
            .into_iter()
            .filter(|flag| flag.contains("proxy"))
            .collect()
        };
        let custom = |scheme: &str, host: &str, port| ProxyServer::Custom {
            scheme: scheme.into(),
            host: host.into(),
            port,
        };
        assert_eq!(proxy_flags(&ProxyServer::System), Vec::<String>::new());
        assert_eq!(proxy_flags(&ProxyServer::Direct), ["--no-proxy-server"]);
        assert_eq!(
            proxy_flags(&custom("http", "proxy.corp.example", 3128)),
            ["--proxy-server=http://proxy.corp.example:3128"]
        );
        assert_eq!(
            proxy_flags(&custom("socks5", "10.0.0.7", 1080)),
            ["--proxy-server=socks5://10.0.0.7:1080"]
        );
        assert_eq!(
            proxy_flags(&custom("https", "2001:db8::1", 443)),
            ["--proxy-server=https://[2001:db8::1]:443"]
        );
        // Nothing may switch off Chrome's direct connections to loopback.
        assert!(
            !arguments(&options(executable))
                .iter()
                .any(|flag| flag.contains("loopback"))
        );
    }

    #[test]
    fn a_kept_profile_gets_a_bounded_cache() {
        let executable = Path::new("chrome");
        let kept = arguments(&LaunchOptions {
            profile: ProfileChoice::Persistent(Path::new("/data/scan-profile/desktop")),
            ..options(executable)
        });
        assert!(kept.contains(&"--disk-cache-size=104857600".to_string()));
        assert!(
            !arguments(&options(executable))
                .iter()
                .any(|flag| flag.starts_with("--disk-cache-size"))
        );
    }

    #[test]
    fn only_processes_started_on_the_profile_are_taken_for_the_browsers() {
        let profile = Path::new("/data/scan-profile/desktop");
        let on_profile =
            |arguments: &[&str]| runs_on_profile(arguments.join("\0").as_bytes(), profile);
        assert!(on_profile(&[
            "chrome",
            "--type=renderer",
            "--user-data-dir=/data/scan-profile/desktop"
        ]));
        assert!(on_profile(&[
            "chrome_crashpad_handler",
            "--database=/data/scan-profile/desktop/Crashpad"
        ]));
        // Someone looking at the profile, a browser on a neighbouring
        // directory, and a browser on the parent are not this browser.
        assert!(!on_profile(&["du", "-sh", "/data/scan-profile/desktop"]));
        assert!(!on_profile(&[
            "tail",
            "-f",
            "/data/scan-profile/desktop/chrome_debug.log"
        ]));
        assert!(!on_profile(&[
            "chrome",
            "--user-data-dir=/data/scan-profile/desktop-old"
        ]));
        assert!(!on_profile(&[
            "chrome",
            "--user-data-dir=/data/scan-profile"
        ]));
        assert!(!on_profile(&[
            "chrome",
            "--note=see /data/scan-profile/desktop"
        ]));
    }

    #[tokio::test]
    async fn a_browser_that_dies_while_starting_is_reported_with_what_it_printed() {
        // A shell rejects Chrome's arguments, complains and exits.
        let mut chrome = Chrome::spawn(options(Path::new("/bin/sh"))).unwrap();

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
        let mut chrome = Chrome::spawn(options(Path::new(&browser.path))).unwrap();
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
