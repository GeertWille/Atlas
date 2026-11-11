//! The Chrome process of one capture: launched on a throwaway profile and
//! guaranteed to be gone, with its profile, when the capture is over.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

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
    Spawn { path: String, source: std::io::Error },
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
        Ok(Self { child, profile: Some(profile), devtools_url: Some(devtools_url) })
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

    /// Kills the whole process tree, waits for it and removes the profile.
    pub(crate) async fn shutdown(mut self) {
        self.kill_process_tree();
        let _ = self.child.wait().await;
        let Some(profile) = self.profile.take() else { return };
        // Helper processes can take a moment to die and release their files.
        let path = profile.path().to_path_buf();
        drop(profile);
        for _ in 0..20 {
            if !path.exists() || std::fs::remove_dir_all(&path).is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn kill_process_tree(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id().and_then(|pid| i32::try_from(pid).ok()) {
            use nix::sys::signal::{Signal, killpg};
            let _ = killpg(nix::unistd::Pid::from_raw(pid), Signal::SIGKILL);
        }
        let _ = self.child.start_kill();
    }
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
            "--disable-default-apps",
            "--disable-extensions",
            "--disable-component-update",
            "--disable-sync",
            "--disable-breakpad",
            "--disable-crash-reporter",
            "--disable-background-timer-throttling",
            "--disable-backgrounding-occluded-windows",
            "--disable-renderer-backgrounding",
            // Without this `navigator.webdriver` is true and ad stacks see a bot.
            "--disable-blink-features=AutomationControlled",
            "--disable-features=Translate,MediaRouter,OptimizationHints,DialMediaRouteProvider",
            "--password-store=basic",
            "--use-mock-keychain",
            "--mute-audio",
        ]
        .map(str::to_string),
    );
    if options.headless {
        arguments.push("--headless=new".to_string());
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
