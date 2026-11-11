//! The macOS app bundle Ranch runs from: whether an update can replace it,
//! and how the updated app starts again under the same Dock tile.
//!
//! The updater swaps the new bundle in at the path of the running one. Tauri's
//! `AppHandle::restart` then runs `open -n` on it while this process is still
//! alive, so LaunchServices starts a second instance and the Dock gives it a
//! tile of its own next to the pinned one, which stays on the old process.
//! [`Relaunch`] hands the start to a detached shell instead: it waits until
//! this process is gone, from the kernel and from LaunchServices, then opens
//! the bundle as a click on the Dock tile would.
//!
//! A bundle on a read-only volume (the disk image) or under App Translocation
//! (the read-only copy macOS runs a quarantined app from) cannot be replaced:
//! [`location_problem`] says so, and the update is refused up front.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// What to tell someone whose Ranch runs from App Translocation.
pub const TRANSLOCATED: &str =
    "Ranch runs from a temporary copy macOS made: move Ranch to Applications in the Finder, open it from there, then update";
/// What to tell someone whose Ranch runs from a read-only volume.
pub const READ_ONLY: &str =
    "Ranch runs from its disk image: copy it to Applications, open it from there, then update";

/// Why the bundle at `bundle` cannot be replaced by an update, `None` when it
/// can. `read_only_volume` is whether the volume holding it is mounted
/// read-only ([`on_read_only_volume`]). Translocation is named first: its
/// copy is read-only too, but moving the app is what fixes it.
pub fn location_problem(bundle: &Path, read_only_volume: bool) -> Option<&'static str> {
    if bundle
        .components()
        .any(|c| c.as_os_str() == "AppTranslocation")
    {
        return Some(TRANSLOCATED);
    }
    read_only_volume.then_some(READ_ONLY)
}

/// Whether the volume holding `path` is mounted read-only. `false` when it
/// cannot be told: the updater then reports the failure itself.
#[cfg(target_os = "macos")]
pub fn on_read_only_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut fs = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `c_path` is NUL-terminated and outlives the call; `fs` is only
    // read once `statfs` has filled it.
    if unsafe { libc::statfs(c_path.as_ptr(), fs.as_mut_ptr()) } != 0 {
        return false;
    }
    let fs = unsafe { fs.assume_init() };
    u64::from(fs.f_flags) & libc::MNT_RDONLY as u64 != 0
}

/// `$1` the pid to outlive, `$2` how many tenths of a second to wait for it at
/// most, `$3` the opener, `$4` the bundle, the rest the app's arguments.
///
/// The pid must be gone from LaunchServices too (`lsappinfo` lists the apps
/// it knows; it is absent off macOS, which reads as gone): until then the
/// Dock still holds the pinned tile for the old process. A plain `open` then
/// starts the app like a click on that tile. If the old process outlives the
/// wait, `open -n` starts the update anyway rather than only bringing the old
/// app forward: two tiles beat no app.
const RELAUNCH_SCRIPT: &str = r#"pid=$1 tenths=$2 opener=$3 bundle=$4
shift 4
while [ "$tenths" -gt 0 ]; do
  if ! kill -0 "$pid" 2>/dev/null && [ -z "$(/usr/bin/lsappinfo find pid="$pid" 2>/dev/null)" ]; then
    exec "$opener" "$bundle" --args "$@"
  fi
  /bin/sleep 0.1
  tenths=$((tenths - 1))
done
exec "$opener" -n "$bundle" --args "$@"
"#;

/// Starting the app at `bundle` again once the process `pid` has exited.
#[derive(Debug, Clone)]
pub struct Relaunch {
    pub pid: u32,
    pub bundle: PathBuf,
    pub args: Vec<OsString>,
    pub opener: PathBuf,
    pub max_wait: Duration,
}

impl Relaunch {
    /// This process, relaunched from `bundle` with `args` by LaunchServices.
    /// Thirty seconds covers a slow quit (the heartbeat cleared, windows
    /// closed) with room to spare.
    pub fn after_this_process(bundle: PathBuf, args: Vec<OsString>) -> Self {
        Self {
            pid: std::process::id(),
            bundle,
            args,
            opener: PathBuf::from("/usr/bin/open"),
            max_wait: Duration::from_secs(30),
        }
    }

    /// The detached shell that waits, then opens. Its own process group and no
    /// stdio, so nothing of this process (a signal to its group, a closed
    /// pipe) stops it once this process has exited.
    pub fn command(&self) -> Command {
        let tenths = (self.max_wait.as_millis() / 100).max(1);
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(RELAUNCH_SCRIPT)
            .arg("ranch-relaunch")
            .arg(self.pid.to_string())
            .arg(tenths.to_string())
            .arg(&self.opener)
            .arg(&self.bundle)
            .args(&self.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        cmd
    }

    /// Starts the shell. The caller exits right after, so the child is never
    /// waited for: launchd adopts it.
    pub fn spawn(&self) -> std::io::Result<Child> {
        self.command().spawn()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    #[test]
    fn a_bundle_in_applications_can_be_replaced() {
        assert_eq!(
            location_problem(Path::new("/Applications/Ranch.app"), false),
            None
        );
        assert_eq!(
            location_problem(Path::new("/Users/someone/Applications/Ranch.app"), false),
            None
        );
    }

    #[test]
    fn a_translocated_bundle_is_named_as_such() {
        let translocated =
            Path::new("/private/var/folders/x1/abc/T/AppTranslocation/0F2E-41C1/d/Ranch.app");
        // Its copy is read-only too: the translocation is what to fix.
        assert_eq!(location_problem(translocated, true), Some(TRANSLOCATED));
        assert_eq!(location_problem(translocated, false), Some(TRANSLOCATED));
    }

    #[test]
    fn a_bundle_on_a_read_only_volume_cannot_be_replaced() {
        assert_eq!(
            location_problem(Path::new("/Volumes/Ranch/Ranch.app"), true),
            Some(READ_ONLY)
        );
        // An app kept on a writable external disk updates like any other.
        assert_eq!(
            location_problem(Path::new("/Volumes/Work/Applications/Ranch.app"), false),
            None
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_volume_is_read_only_and_the_temp_dir_is_not() {
        // The sealed system volume, where /System lives, is mounted read-only.
        assert!(on_read_only_volume(Path::new("/System")));
        assert!(!on_read_only_volume(&std::env::temp_dir()));
        assert!(!on_read_only_volume(Path::new("/nowhere/at/all")));
    }

    /// An opener that writes its arguments, one per line, then `done`.
    fn fake_opener(dir: &Path) -> (PathBuf, PathBuf) {
        let log = dir.join("opened");
        let opener = dir.join("open");
        std::fs::write(
            &opener,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do echo \"$a\"; done > '{}.tmp'\nmv '{}.tmp' '{}'\n",
                log.display(),
                log.display(),
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&opener, std::fs::Permissions::from_mode(0o755)).unwrap();
        (opener, log)
    }

    fn wait_for(path: &Path, limit: Duration) -> Option<String> {
        let start = Instant::now();
        while start.elapsed() < limit {
            if let Ok(s) = std::fs::read_to_string(path) {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn the_app_is_opened_once_the_old_process_is_gone_without_a_new_instance() {
        let dir = tempfile::tempdir().unwrap();
        let (opener, log) = fake_opener(dir.path());
        // The "old app": a process that lives for a while, reaped by a thread
        // as launchd would reap the real one.
        let mut old = Command::new("/bin/sleep").arg("0.6").spawn().unwrap();
        let pid = old.id();
        let reaper = std::thread::spawn(move || {
            old.wait().unwrap();
            Instant::now()
        });

        let relaunch = Relaunch {
            pid,
            bundle: PathBuf::from("/Applications/Ranch.app"),
            args: vec!["--no-update-check".into(), "two words".into()],
            opener,
            max_wait: Duration::from_secs(20),
        };
        let mut shell = relaunch.spawn().unwrap();

        let opened = wait_for(&log, Duration::from_secs(10)).expect("the opener ran");
        let seen_at = Instant::now();
        let gone_at = reaper.join().unwrap();
        assert!(
            seen_at >= gone_at,
            "opened only after the old process exited"
        );
        assert_eq!(
            opened.lines().collect::<Vec<_>>(),
            [
                "/Applications/Ranch.app",
                "--args",
                "--no-update-check",
                "two words"
            ],
            "a plain open, so the Dock gives the app its pinned tile"
        );
        assert!(shell.wait().unwrap().success());
    }

    #[test]
    fn the_app_is_not_opened_while_the_old_process_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (opener, log) = fake_opener(dir.path());
        let mut old = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let relaunch = Relaunch {
            pid: old.id(),
            bundle: PathBuf::from("/Applications/Ranch.app"),
            args: vec![],
            opener,
            max_wait: Duration::from_secs(20),
        };
        let mut shell = relaunch.spawn().unwrap();
        assert_eq!(wait_for(&log, Duration::from_millis(800)), None);
        old.kill().unwrap();
        old.wait().unwrap();
        assert!(wait_for(&log, Duration::from_secs(10)).is_some());
        assert!(shell.wait().unwrap().success());
    }

    #[test]
    fn an_old_process_that_outlives_the_wait_still_gets_the_update_started() {
        let dir = tempfile::tempdir().unwrap();
        let (opener, log) = fake_opener(dir.path());
        let mut old = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let relaunch = Relaunch {
            pid: old.id(),
            bundle: PathBuf::from("/Applications/Ranch.app"),
            args: vec![],
            opener,
            max_wait: Duration::from_millis(300),
        };
        let mut shell = relaunch.spawn().unwrap();
        let opened = wait_for(&log, Duration::from_secs(10)).expect("the opener ran");
        assert_eq!(
            opened.lines().collect::<Vec<_>>(),
            ["-n", "/Applications/Ranch.app", "--args"]
        );
        assert!(shell.wait().unwrap().success());
        old.kill().unwrap();
        old.wait().unwrap();
    }

    #[test]
    fn this_process_is_relaunched_by_launch_services() {
        let r = Relaunch::after_this_process(PathBuf::from("/Applications/Ranch.app"), vec![]);
        assert_eq!(r.pid, std::process::id());
        assert_eq!(r.opener, Path::new("/usr/bin/open"));
        assert!(r.max_wait >= Duration::from_secs(10));
    }
}
