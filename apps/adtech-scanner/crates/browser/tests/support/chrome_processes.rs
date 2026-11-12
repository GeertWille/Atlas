//! Finds the browser a capture started, and everything that browser spawned,
//! by what they leave in `/proc`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PROFILE_FLAG: &str = "--user-data-dir=";

struct Process {
    pid: u32,
    name: String,
    parent: u32,
    group: u32,
    is_zombie: bool,
}

fn processes() -> Vec<Process> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            // "pid (comm) state ppid pgrp ..."; comm may contain spaces.
            let (before, after) = stat.rsplit_once(')')?;
            let mut fields = after.split_whitespace();
            let is_zombie = fields.next()? == "Z";
            Some(Process {
                pid,
                name: before.split_once('(')?.1.to_string(),
                parent: fields.next()?.parse().ok()?,
                group: fields.next()?.parse().ok()?,
                is_zombie,
            })
        })
        .collect()
}

pub fn command_line(pid: u32) -> Vec<String> {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .unwrap_or_default()
        .split(|byte| *byte == 0)
        .map(|argument| String::from_utf8_lossy(argument).into_owned())
        .collect()
}

fn profile_of(pid: u32) -> Option<PathBuf> {
    command_line(pid)
        .iter()
        .find_map(|argument| argument.strip_prefix(PROFILE_FLAG).map(PathBuf::from))
}

fn mentions(pid: u32, text: &str) -> bool {
    command_line(pid)
        .iter()
        .any(|argument| argument.contains(text))
}

/// Live children of `parent` whose executable is called `name`.
pub fn children_named(parent: u32, name: &str) -> Vec<u32> {
    processes()
        .into_iter()
        .filter(|process| process.parent == parent && process.name == name && !process.is_zombie)
        .map(|process| process.pid)
        .collect()
}

/// A browser launched by a capture: its process group and profile.
pub struct Browser {
    /// Also the process id of the browser's main process.
    pub group: u32,
    pub profile: PathBuf,
}

impl Browser {
    /// Waits for a capture running in the process `parent` to launch its
    /// browser, on a throwaway profile.
    pub async fn launched_by(parent: u32) -> Self {
        let browser = Self::child_of(parent).await;
        assert!(
            browser
                .profile
                .to_string_lossy()
                .contains("adscan-profile-"),
            "{:?}",
            browser.profile
        );
        browser
    }

    /// The same for a capture on the scan profile that is kept.
    pub async fn launched_on_kept_profile(parent: u32, profile: &Path) -> Self {
        let browser = Self::child_of(parent).await;
        assert_eq!(browser.profile, profile);
        browser
    }

    async fn child_of(parent: u32) -> Self {
        for _ in 0..1000 {
            let children = processes()
                .into_iter()
                .filter(|process| process.parent == parent);
            if let Some((group, profile)) = children
                .filter_map(|process| Some((process.pid, profile_of(process.pid)?)))
                .next()
            {
                return Self { group, profile };
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no browser was launched");
    }

    /// Live processes of the browser: its process group (browser, zygotes,
    /// GPU, renderers) plus anything else started with its profile path.
    pub fn live_processes(&self) -> usize {
        let profile = self.profile.to_string_lossy();
        processes()
            .iter()
            .filter(|process| !process.is_zombie)
            .filter(|process| process.group == self.group || mentions(process.pid, &profile))
            .count()
    }

    /// The command line of every live process of the browser, each as one
    /// string.
    pub fn command_lines(&self) -> Vec<String> {
        let profile = self.profile.to_string_lossy();
        processes()
            .iter()
            .filter(|process| !process.is_zombie)
            .filter(|process| process.group == self.group || mentions(process.pid, &profile))
            .map(|process| command_line(process.pid).join(" "))
            .collect()
    }

    /// Waits for every process of the browser to be gone. Says nothing
    /// about the profile.
    pub async fn assert_exited(&self, within: Duration) {
        let deadline = Instant::now() + within;
        while self.live_processes() > 0 {
            assert!(
                Instant::now() < deadline,
                "{} Chrome processes are still alive",
                self.live_processes()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits for the processes and the throwaway profile to be gone.
    pub async fn assert_gone(&self, within: Duration) {
        let deadline = Instant::now() + within;
        loop {
            let live = self.live_processes();
            if live == 0 && !self.profile.exists() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{live} Chrome processes are still alive, profile {} exists: {}",
                self.profile.display(),
                self.profile.exists()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
