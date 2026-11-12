//! A watchdog for the case nothing in this process can handle: the process
//! being killed. It is a shell that waits on a pipe from this process, in its
//! own process group so that a Ctrl+C aimed at the app does not reach it. The
//! pipe reports end of file when this process is gone, whatever ended it, and
//! the shell then kills the browser's process group and removes its profile,
//! unless the profile is one the user keeps.
//!
//! macOS has no way to tie a child's life to its parent's, and a browser that
//! hangs does not notice its DevTools pipe closing; the watchdog covers both.

use std::io::{PipeWriter, Write};
use std::path::Path;
use std::process::Stdio;

use tokio::process::Command;

/// `read` succeeds when the owner lets go on purpose and fails at end of
/// file. Dying browser processes can write into the profile for a moment
/// longer, hence the retries.
const SCRIPT: &str = r#"
trap '' HUP INT TERM
if read -r released; then exit 0; fi
kill -s KILL -- "-$ADSCAN_BROWSER_GROUP" 2>/dev/null
[ -n "$ADSCAN_BROWSER_PROFILE" ] || exit 0
for attempt in 1 2 3 4 5; do
    rm -rf -- "$ADSCAN_BROWSER_PROFILE" 2>/dev/null
    [ -e "$ADSCAN_BROWSER_PROFILE" ] || exit 0
    sleep 1
done
"#;

/// Dropping the value without calling [`Watchdog::disarm`] is what the death
/// of this process looks like to the watchdog: it kills and cleans up.
pub(crate) struct Watchdog {
    leash: PipeWriter,
}

impl Watchdog {
    /// Starts watching over the browser whose process group is `group`.
    /// `disposable_profile` is the directory to delete with it, if any.
    pub(crate) fn guard(group: u32, disposable_profile: Option<&Path>) -> std::io::Result<Self> {
        let (held_by_shell, leash) = std::io::pipe()?;
        // Not held on to: the runtime reaps the shell when it exits.
        shell(group, disposable_profile)
            .stdin(held_by_shell)
            .spawn()?;
        Ok(Self { leash })
    }

    /// Tells the watchdog that the browser was shut down in an orderly way,
    /// so there is nothing for it to do.
    pub(crate) fn disarm(mut self) {
        let _ = self.leash.write_all(b"\n");
    }
}

fn shell(group: u32, disposable_profile: Option<&Path>) -> Command {
    let mut shell = Command::new("/bin/sh");
    shell
        .args(["-c", SCRIPT])
        // In the environment, not as arguments: every process with the
        // profile path on its command line is taken for one of the browser's.
        .env("ADSCAN_BROWSER_GROUP", group.to_string())
        .env(
            "ADSCAN_BROWSER_PROFILE",
            disposable_profile.unwrap_or(Path::new("")),
        )
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    shell
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    use tokio::io::AsyncReadExt;
    use tokio::process::{Child, ChildStdout};

    use super::*;

    /// Stands in for a browser: a process group of two processes that would
    /// run for a long time.
    struct ProcessGroup {
        leader: Child,
        /// Open in every process of the group, so it reports end of file
        /// once all of them are dead, reaped or not.
        held_open_by_the_group: ChildStdout,
    }

    impl ProcessGroup {
        fn start() -> Self {
            let mut leader = Command::new("/bin/sh")
                .args(["-c", "sleep 600 & sleep 600"])
                .stdout(Stdio::piped())
                .process_group(0)
                .spawn()
                .unwrap();
            let held_open_by_the_group = leader.stdout.take().unwrap();
            Self {
                leader,
                held_open_by_the_group,
            }
        }

        fn id(&self) -> u32 {
            self.leader.id().unwrap()
        }

        async fn dies_within(&mut self, patience: Duration) -> bool {
            let end_of_file = self.held_open_by_the_group.read_u8();
            tokio::time::timeout(patience, end_of_file).await.is_ok()
        }
    }

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            if let Some(id) = self.leader.id() {
                let _ = kill(Pid::from_raw(-id.cast_signed()), Signal::SIGKILL);
            }
        }
    }

    fn profile_with_data() -> tempfile::TempDir {
        let profile = tempfile::tempdir().unwrap();
        std::fs::create_dir(profile.path().join("Default")).unwrap();
        std::fs::write(profile.path().join("Default/Cookies"), "data").unwrap();
        profile
    }

    async fn is_removed_within(profile: &Path, patience: Duration) -> bool {
        let deadline = Instant::now() + patience;
        while profile.exists() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        !profile.exists()
    }

    #[tokio::test]
    async fn losing_the_owner_kills_the_whole_group_and_removes_the_profile() {
        let mut group = ProcessGroup::start();
        let profile = profile_with_data();
        let watchdog = Watchdog::guard(group.id(), Some(profile.path())).unwrap();
        assert!(!group.dies_within(Duration::from_millis(300)).await);
        assert!(profile.path().exists());

        drop(watchdog);

        assert!(group.dies_within(Duration::from_secs(5)).await);
        let leader = group.leader.wait().await.unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&leader),
            Some(Signal::SIGKILL as i32)
        );
        assert!(is_removed_within(profile.path(), Duration::from_secs(5)).await);
    }

    #[tokio::test]
    async fn a_disarmed_watchdog_touches_nothing() {
        let mut group = ProcessGroup::start();
        let profile = profile_with_data();
        let watchdog = Watchdog::guard(group.id(), Some(profile.path())).unwrap();

        watchdog.disarm();

        assert!(!group.dies_within(Duration::from_millis(500)).await);
        assert!(profile.path().join("Default/Cookies").exists());
    }

    #[tokio::test]
    async fn without_a_disposable_profile_the_group_is_killed_and_the_watchdog_is_done() {
        let mut group = ProcessGroup::start();
        let (held_by_shell, leash) = std::io::pipe().unwrap();
        let mut watchdog = shell(group.id(), None)
            .stdin(held_by_shell)
            .spawn()
            .unwrap();

        drop(leash);

        assert!(group.dies_within(Duration::from_secs(5)).await);
        let finished = tokio::time::timeout(Duration::from_millis(900), watchdog.wait());
        assert!(
            finished.await.unwrap().unwrap().success(),
            "no removal loop runs"
        );
    }

    /// The signals that end the app must not end the watchdog before it has
    /// done its work.
    #[tokio::test]
    async fn termination_signals_do_not_stop_the_watchdog() {
        let mut group = ProcessGroup::start();
        let profile = profile_with_data();
        let (held_by_shell, leash) = std::io::pipe().unwrap();
        let mut watchdog = shell(group.id(), Some(profile.path()))
            .stdin(held_by_shell)
            .spawn()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let watchdog_id = Pid::from_raw(watchdog.id().unwrap().cast_signed());
        for signal in [Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP] {
            kill(watchdog_id, signal).unwrap();
        }
        assert!(!group.dies_within(Duration::from_millis(300)).await);
        assert!(watchdog.try_wait().unwrap().is_none());

        drop(leash);

        assert!(group.dies_within(Duration::from_secs(5)).await);
        assert!(is_removed_within(profile.path(), Duration::from_secs(5)).await);
        assert!(watchdog.wait().await.unwrap().success());
    }
}
