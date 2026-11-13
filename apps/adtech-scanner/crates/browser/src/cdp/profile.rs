//! Chrome profiles: the throwaway profile of one launch, the removal of
//! throwaway profiles whose owning process died before it could remove them,
//! and the browser profile a user chose to keep between scans.
//!
//! A profile is owned through an advisory lock on a file inside it. The
//! operating system releases the lock when the owning process ends, however it
//! ends, so "nobody holds the lock" means "nobody is using this profile"
//! without relying on process ids, which get reused.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const PREFIX: &str = "adscan-profile-";
/// A profile is created under this prefix and renamed once it is locked, so
/// that no unlocked profile of a live launch is ever visible under `PREFIX`.
const STAGING_PREFIX: &str = "adscan-starting-";
const LOCK_FILE: &str = "adscan-owner.lock";
/// How old a profile without a lock file must be before it counts as
/// abandoned: longer than any capture lasts. Such profiles come from versions
/// of the app that did not lock them, and from dying Chrome processes that
/// recreate the directory after it was removed.
const UNLOCKED_PROFILE_GRACE: Duration = Duration::from_secs(60 * 60);

pub(crate) struct Profile {
    path: PathBuf,
    /// Holds the lock for as long as the profile is in use.
    _owner: File,
}

impl Profile {
    pub(crate) fn create_in(root: &Path) -> std::io::Result<Self> {
        let staging = tempfile::Builder::new()
            .prefix(STAGING_PREFIX)
            .tempdir_in(root)?;
        let owner = File::create(staging.path().join(LOCK_FILE))?;
        owner.lock()?;
        let unique_suffix = staging
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(STAGING_PREFIX))
            .ok_or_else(|| std::io::Error::other("unexpected temporary directory name"))?;
        let path = root.join(format!("{PREFIX}{unique_suffix}"));
        std::fs::rename(staging.path(), &path)?;
        // Renamed away; there is nothing left for `staging` to delete.
        let _ = staging.keep();
        Ok(Self {
            path,
            _owner: owner,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A profile that outlives the launch: cookies, storage and cache stay for
/// the next scan. Nothing in this crate deletes it except [`clear`], which
/// the user asks for.
pub(crate) struct PersistentProfile {
    path: PathBuf,
    /// Holds the lock for as long as the profile is in use.
    _owner: File,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PersistentProfileError {
    #[error("the browser profile is in use by another scan")]
    Busy,
    #[error("the browser profile could not be opened: {0}")]
    Io(#[from] std::io::Error),
}

const SINGLETON_SOCKET: &str = "SingletonSocket";
const SINGLETON_COOKIE: &str = "SingletonCookie";
/// What a browser that was killed leaves in its profile. A new browser takes
/// them for a running instance, hands its start over to it and exits.
const SINGLETON_FILES: [&str; 3] = ["SingletonLock", SINGLETON_SOCKET, SINGLETON_COOKIE];

/// Chrome keeps the socket that makes it the one browser on a profile in a
/// directory of its own in the temp dir, because the path of a socket may
/// only be so long, and links to it from the profile. A browser that exits
/// removes that directory; one that is killed, as the browser of every scan
/// on a throwaway profile is, leaves it. This removes the directory the
/// profile links to: the two files Chrome puts there, by name, and then the
/// directory if that emptied it. A link that leads anywhere else loses
/// nothing but files of those names.
pub(crate) fn remove_singleton_directory(profile: &Path) {
    let Ok(socket) = std::fs::read_link(profile.join(SINGLETON_SOCKET)) else {
        return;
    };
    let Some(directory) = socket.parent().filter(|directory| directory.is_absolute()) else {
        return;
    };
    for name in [SINGLETON_SOCKET, SINGLETON_COOKIE] {
        let _ = std::fs::remove_file(directory.join(name));
    }
    let _ = std::fs::remove_dir(directory);
}

impl PersistentProfile {
    /// Takes the profile at `path` for one launch, creating it when needed.
    pub(crate) fn open(path: &Path) -> Result<Self, PersistentProfileError> {
        let owner = lock(path, Missing::Create)?.ok_or(PersistentProfileError::Busy)?;
        // The lock file's age is when the profile was last used.
        owner.set_modified(SystemTime::now())?;
        remove_singleton_directory(path);
        for name in SINGLETON_FILES {
            // `SingletonLock` is a dangling symlink, which `exists` denies.
            let _ = std::fs::remove_file(path.join(name));
        }
        Ok(Self {
            path: path.to_path_buf(),
            _owner: owner,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

/// [`inspect`] holds the lock for an instant to see whether anyone else does.
/// A scan that starts in that instant must not be told the profile is busy.
const LOCK_ATTEMPTS: u32 = 5;
const LOCK_RETRY_PAUSE: Duration = Duration::from_millis(20);

/// What taking the lock of a profile that is not on disk does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Missing {
    Create,
    /// There is nothing to own: `Ok(None)`.
    Leave,
}

fn lock(profile: &Path, missing: Missing) -> Result<Option<File>, PersistentProfileError> {
    let lock_file = profile.join(LOCK_FILE);
    for attempt in 1..=LOCK_ATTEMPTS {
        if missing == Missing::Create {
            std::fs::create_dir_all(profile)?;
        } else if !profile.exists() {
            return Ok(None);
        }
        let owner = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_file)?;
        match owner.try_lock() {
            // Whoever held the lock before may have deleted the profile
            // ([`clear`]): a lock on a file that is no longer the profile's
            // lock file keeps nobody else out.
            Ok(()) if is_the_file_at(&owner, &lock_file) => return Ok(Some(owner)),
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                if attempt < LOCK_ATTEMPTS {
                    std::thread::sleep(LOCK_RETRY_PAUSE);
                }
            }
            Err(TryLockError::Error(error)) => return Err(error.into()),
        }
    }
    Err(PersistentProfileError::Busy)
}

fn is_the_file_at(open: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (open.metadata(), std::fs::metadata(path)) {
        (Ok(open), Ok(at_path)) => open.dev() == at_path.dev() && open.ino() == at_path.ino(),
        _ => false,
    }
}

/// What is on disk for the persistent profile at `path`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PersistentProfileState {
    pub exists: bool,
    pub size_bytes: u64,
    pub last_used: Option<SystemTime>,
    pub in_use: bool,
}

pub(crate) fn inspect(path: &Path) -> PersistentProfileState {
    let owner = File::open(path.join(LOCK_FILE));
    PersistentProfileState {
        exists: path.is_dir(),
        size_bytes: size_of(path),
        last_used: owner
            .as_ref()
            .ok()
            .and_then(|owner| owner.metadata().ok()?.modified().ok()),
        in_use: owner.is_ok_and(|owner| matches!(owner.try_lock(), Err(TryLockError::WouldBlock))),
    }
}

fn size_of(directory: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => size_of(&entry.path()),
            Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |metadata| metadata.len()),
            _ => 0,
        })
        .sum()
}

/// Deletes the persistent profiles at `paths`, unless a scan is using one of
/// them: then none is deleted.
pub(crate) fn clear(paths: &[PathBuf]) -> Result<(), PersistentProfileError> {
    let mut owned = Vec::new();
    for path in paths {
        if let Some(owner) = lock(path, Missing::Leave)? {
            owned.push((path, owner));
        }
    }
    for (path, _owner) in &owned {
        // The lock file goes last: while it is there and locked, a scan that
        // starts now waits or is told the profile is busy, and never opens a
        // profile that is half deleted.
        for entry in std::fs::read_dir(path)?.flatten() {
            if entry.file_name() == LOCK_FILE {
                continue;
            }
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(entry.path())?;
            } else {
                std::fs::remove_file(entry.path())?;
            }
        }
        std::fs::remove_file(path.join(LOCK_FILE))?;
        // A scan may have taken the profile the moment its lock file went;
        // the directory is then that scan's and stays.
        let _ = std::fs::remove_dir(path);
    }
    Ok(())
}

/// Removes the profiles in `root` that no live process owns and returns how
/// many were removed. Profiles of running captures, in this process or in
/// another instance of the app, are left alone.
pub(crate) fn remove_abandoned(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            let is_profile = entry.file_name().to_string_lossy().starts_with(PREFIX);
            // `file_type` does not follow symlinks: a link is never a profile.
            is_profile && entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .filter(|entry| is_abandoned(&entry.path()))
        .filter(|entry| {
            remove_singleton_directory(&entry.path());
            std::fs::remove_dir_all(entry.path()).is_ok()
        })
        .count()
}

fn is_abandoned(profile: &Path) -> bool {
    match File::open(profile.join(LOCK_FILE)) {
        Ok(owner) => owner.try_lock().is_ok(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => std::fs::metadata(profile)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| {
                modified
                    .elapsed()
                    .is_ok_and(|age| age > UNLOCKED_PROFILE_GRACE)
            }),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_names(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn name_of(profile: &Profile) -> String {
        profile
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    /// What a process that was killed leaves behind: the directory with its
    /// lock file, and nobody holding the lock.
    fn left_behind_by_a_dead_owner(root: &Path, name: &str) -> PathBuf {
        let path = root.join(format!("{PREFIX}{name}"));
        std::fs::create_dir_all(path.join("Default")).unwrap();
        std::fs::write(path.join("Default/Cookies"), "data").unwrap();
        File::create(path.join(LOCK_FILE)).unwrap();
        path
    }

    fn age(directory: &Path, age: Duration) {
        File::open(directory)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    #[test]
    fn a_profile_is_locked_before_it_appears_under_its_final_name() {
        let root = tempfile::tempdir().unwrap();

        let profile = Profile::create_in(root.path()).unwrap();

        assert_eq!(profile_names(root.path()), [name_of(&profile)]);
        assert!(name_of(&profile).starts_with(PREFIX));
        let lock = File::open(profile.path().join(LOCK_FILE)).unwrap();
        assert!(matches!(
            lock.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
    }

    #[test]
    fn dropping_a_profile_removes_it() {
        let root = tempfile::tempdir().unwrap();
        let profile = Profile::create_in(root.path()).unwrap();
        std::fs::create_dir(profile.path().join("Default")).unwrap();

        drop(profile);

        assert!(profile_names(root.path()).is_empty());
    }

    #[test]
    fn profiles_of_dead_owners_are_removed_and_live_ones_are_kept() {
        let root = tempfile::tempdir().unwrap();
        let live = Profile::create_in(root.path()).unwrap();
        let another_live = Profile::create_in(root.path()).unwrap();
        let dead = left_behind_by_a_dead_owner(root.path(), "killed");
        let also_dead = left_behind_by_a_dead_owner(root.path(), "crashed");

        assert_eq!(remove_abandoned(root.path()), 2);

        assert!(!dead.exists() && !also_dead.exists());
        let mut kept = [name_of(&live), name_of(&another_live)];
        kept.sort();
        assert_eq!(profile_names(root.path()), kept);
        assert_eq!(remove_abandoned(root.path()), 0, "nothing more to remove");
    }

    #[test]
    fn a_profile_without_a_lock_file_is_only_removed_once_it_is_old() {
        let root = tempfile::tempdir().unwrap();
        let fresh = root.path().join(format!("{PREFIX}fresh"));
        let nearly_old = root.path().join(format!("{PREFIX}nearly-old"));
        let old = root.path().join(format!("{PREFIX}old"));
        for profile in [&fresh, &nearly_old, &old] {
            std::fs::create_dir_all(profile.join("Default")).unwrap();
        }
        age(
            &nearly_old,
            UNLOCKED_PROFILE_GRACE - Duration::from_secs(60),
        );
        age(&old, UNLOCKED_PROFILE_GRACE + Duration::from_secs(60));

        assert_eq!(remove_abandoned(root.path()), 1);

        assert!(fresh.exists() && nearly_old.exists() && !old.exists());
    }

    #[test]
    fn only_profile_directories_are_ever_touched() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let precious = elsewhere.path().join("precious");
        std::fs::create_dir(&precious).unwrap();
        std::fs::write(precious.join("file"), "keep").unwrap();
        age(&precious, UNLOCKED_PROFILE_GRACE * 2);

        let other_app = root.path().join("other-app-profile-1");
        std::fs::create_dir(&other_app).unwrap();
        age(&other_app, UNLOCKED_PROFILE_GRACE * 2);
        let starting = root.path().join(format!("{STAGING_PREFIX}abc"));
        std::fs::create_dir(&starting).unwrap();
        File::create(starting.join(LOCK_FILE)).unwrap();
        let file = root.path().join(format!("{PREFIX}a-file"));
        std::fs::write(&file, "not a directory").unwrap();
        let link = root.path().join(format!("{PREFIX}a-link"));
        std::os::unix::fs::symlink(&precious, &link).unwrap();

        assert_eq!(remove_abandoned(root.path()), 0);

        assert!(other_app.exists() && starting.exists() && file.exists());
        assert!(link.is_symlink() && precious.join("file").exists());
    }

    #[test]
    fn a_persistent_profile_is_created_locked_and_left_on_disk_when_dropped() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("scan-profile/desktop");

        let profile = PersistentProfile::open(&path).unwrap();
        std::fs::create_dir(profile.path().join("Default")).unwrap();
        std::fs::write(profile.path().join("Default/Cookies"), "data").unwrap();
        assert!(matches!(
            PersistentProfile::open(&path),
            Err(PersistentProfileError::Busy)
        ));
        let in_use = inspect(&path);
        assert!(in_use.exists && in_use.in_use);

        drop(profile);

        assert!(path.join("Default/Cookies").exists());
        assert!(!inspect(&path).in_use);
        assert!(PersistentProfile::open(&path).is_ok(), "free again");
    }

    #[test]
    fn opening_a_persistent_profile_removes_what_a_killed_browser_left() {
        let root = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("gone-host-12345", root.path().join("SingletonLock")).unwrap();
        std::fs::write(root.path().join("SingletonCookie"), "").unwrap();
        std::fs::write(root.path().join("Local State"), "{}").unwrap();

        let _profile = PersistentProfile::open(root.path()).unwrap();

        assert!(!root.path().join("SingletonLock").is_symlink());
        assert!(!root.path().join("SingletonCookie").exists());
        assert!(root.path().join("Local State").exists());
    }

    /// What a killed browser leaves outside its profile: the directory of
    /// its socket in the temp dir, linked from the profile.
    fn singleton_directory_of(profile: &Path, temp_dir: &Path) -> PathBuf {
        let directory = temp_dir.join("org.chromium.Chromium.Ab12Cd");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join(SINGLETON_SOCKET), "").unwrap();
        std::os::unix::fs::symlink("1234567890", directory.join(SINGLETON_COOKIE)).unwrap();
        std::os::unix::fs::symlink(
            directory.join(SINGLETON_SOCKET),
            profile.join(SINGLETON_SOCKET),
        )
        .unwrap();
        directory
    }

    #[test]
    fn the_socket_directory_a_killed_browser_left_in_the_temp_dir_goes_with_its_profile() {
        let temp_dir = tempfile::tempdir().unwrap();
        let kept = temp_dir.path().join("kept");
        std::fs::create_dir(&kept).unwrap();
        let of_kept = singleton_directory_of(&kept, temp_dir.path());
        drop(PersistentProfile::open(&kept).unwrap());
        assert!(!of_kept.exists(), "removed when the profile is next used");

        let abandoned = left_behind_by_a_dead_owner(temp_dir.path(), "dead");
        let of_abandoned = singleton_directory_of(&abandoned, temp_dir.path());
        assert_eq!(remove_abandoned(temp_dir.path()), 1);
        assert!(!of_abandoned.exists(), "removed with an abandoned profile");
    }

    #[test]
    fn only_the_browsers_own_two_files_are_removed_where_the_profile_links_to() {
        let temp_dir = tempfile::tempdir().unwrap();
        let profile = temp_dir.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        let directory = singleton_directory_of(&profile, temp_dir.path());
        std::fs::write(directory.join("someone-elses"), "data").unwrap();

        remove_singleton_directory(&profile);

        assert!(!directory.join(SINGLETON_SOCKET).exists());
        assert!(!directory.join(SINGLETON_COOKIE).is_symlink());
        assert!(directory.join("someone-elses").exists());

        // A link that does not say where in the file system it leads.
        let relative = temp_dir.path().join("relative");
        std::fs::create_dir(&relative).unwrap();
        std::os::unix::fs::symlink(SINGLETON_SOCKET, relative.join(SINGLETON_SOCKET)).unwrap();
        std::fs::write(relative.join(SINGLETON_COOKIE), "").unwrap();
        remove_singleton_directory(&relative);
        assert!(relative.join(SINGLETON_COOKIE).exists());
        remove_singleton_directory(&temp_dir.path().join("missing"));
    }

    #[test]
    fn a_persistent_profile_reports_its_size_and_when_it_was_last_used() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("profile");
        assert_eq!(
            inspect(&path),
            PersistentProfileState {
                exists: false,
                size_bytes: 0,
                last_used: None,
                in_use: false
            }
        );

        let before = SystemTime::now() - Duration::from_secs(1);
        let profile = PersistentProfile::open(&path).unwrap();
        std::fs::create_dir_all(path.join("Default/Cache")).unwrap();
        std::fs::write(path.join("Default/Cookies"), [0u8; 1_000]).unwrap();
        std::fs::write(path.join("Default/Cache/entry"), [0u8; 234]).unwrap();
        drop(profile);

        let state = inspect(&path);
        assert!(state.exists && !state.in_use);
        assert_eq!(state.size_bytes, 1_234);
        assert!(state.last_used.unwrap() > before);
    }

    #[test]
    fn clearing_removes_persistent_profiles_unless_one_of_them_is_in_use() {
        let root = tempfile::tempdir().unwrap();
        let both = [root.path().join("desktop"), root.path().join("mobile")];
        assert!(clear(&both).is_ok(), "nothing to clear is not an error");

        let desktop = PersistentProfile::open(&both[0]).unwrap();
        drop(PersistentProfile::open(&both[1]).unwrap());
        for profile in &both {
            std::fs::write(profile.join("Cookies"), "data").unwrap();
        }
        assert!(matches!(clear(&both), Err(PersistentProfileError::Busy)));
        assert!(
            both.iter().all(|profile| profile.join("Cookies").exists()),
            "neither is touched while one is in use"
        );

        drop(desktop);
        clear(&both).unwrap();
        assert!(both.iter().all(|profile| !profile.exists()));
    }

    /// Two instances of the app: one clears the profile while the other
    /// starts a scan on it. The scan must end up owning the profile that is
    /// on disk, not a lock file the clearing deleted.
    #[test]
    fn a_scan_that_starts_while_the_profile_is_cleared_owns_the_new_profile() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("desktop");
        drop(PersistentProfile::open(&path).unwrap());
        std::fs::write(path.join("Cookies"), "data").unwrap();

        // What `clear` does, with the scan starting in the middle of it.
        let clearing = lock(&path, Missing::Leave).unwrap().unwrap();
        let waiting_for_the_lock = File::options()
            .write(true)
            .open(path.join(LOCK_FILE))
            .unwrap();
        std::fs::remove_dir_all(&path).unwrap();
        drop(clearing);
        waiting_for_the_lock.try_lock().unwrap();
        assert!(
            !is_the_file_at(&waiting_for_the_lock, &path.join(LOCK_FILE)),
            "a lock on the deleted file owns nothing"
        );
        drop(waiting_for_the_lock);

        let scan = PersistentProfile::open(&path).unwrap();
        assert!(matches!(
            PersistentProfile::open(&path),
            Err(PersistentProfileError::Busy)
        ));
        assert!(matches!(
            clear(std::slice::from_ref(&path)),
            Err(PersistentProfileError::Busy)
        ));
        drop(scan);
    }

    #[test]
    fn clearing_removes_everything_and_the_lock_file_last() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("desktop");
        drop(PersistentProfile::open(&path).unwrap());
        std::fs::create_dir_all(path.join("Default/Cache")).unwrap();
        std::fs::write(path.join("Default/Cache/entry"), "data").unwrap();
        std::os::unix::fs::symlink("gone-host-1", path.join("SingletonLock")).unwrap();

        clear(std::slice::from_ref(&path)).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn the_sweep_of_abandoned_profiles_never_takes_a_persistent_one() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("scan-profile");
        drop(PersistentProfile::open(&path).unwrap());
        age(&path, UNLOCKED_PROFILE_GRACE * 2);

        assert_eq!(remove_abandoned(root.path()), 0);
        assert_eq!(remove_abandoned(&path), 0);

        assert!(path.join(LOCK_FILE).exists());
    }

    #[test]
    fn a_missing_root_is_nothing_to_clean() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(remove_abandoned(&root.path().join("missing")), 0);
    }
}
