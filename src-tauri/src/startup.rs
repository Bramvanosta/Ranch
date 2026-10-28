//! What the desktop app does before its window opens, and what it does when
//! that goes wrong: the log file, a database that does not open, a key of
//! the seals that is gone.

use std::path::{Path, PathBuf};
use std::process::Command;

use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::adapters::sqlite::Db;
use crate::app_state::AppState;
use crate::application::ports::settings::SafetySetting;
use crate::domain::errors::RanchError;

const BUNDLE_IDENTIFIER: &str = "dev.ranch.app";
const LOG_FILE: &str = "ranch.log";
/// Past this size at launch, the log is set aside (one previous file kept).
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

// ── the log ─────────────────────────────────────────────────────────────────

/// Log to the terminal and to `ranch.log` in the OS's log folder for the app
/// (Tauri's `app_log_dir`, worked out here because Tauri has not started:
/// a database that does not open is logged too). Respects `RUST_LOG`
/// (e.g. `RUST_LOG=ranch=debug`), INFO by default. Returns the log file.
pub fn init_logging() -> Option<PathBuf> {
    let log = open_log_file();
    let file_layer = log.as_ref().and_then(|(_, file)| {
        let file = file.try_clone().ok()?;
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file)),
        )
    });
    // `try_init` so we don't panic if a host process (test harness,
    // embedded scenario) already installed a subscriber.
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .try_init();

    // A panic goes to the log as well, then on as before.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("{info}");
        default_hook(info);
    }));

    log.map(|(path, _)| path)
}

fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Logs")
                .join(BUNDLE_IDENTIFIER)
        })
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .map(|base| base.join(BUNDLE_IDENTIFIER).join("logs"))
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join(BUNDLE_IDENTIFIER).join("logs"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

fn open_log_file() -> Option<(PathBuf, std::fs::File)> {
    let dir = log_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(LOG_FILE);
    let too_big = std::fs::metadata(&path)
        .map(|meta| meta.len() > MAX_LOG_BYTES)
        .unwrap_or(false);
    if too_big {
        let _ = std::fs::rename(&path, dir.join(format!("{LOG_FILE}.1")));
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    Some((path, file))
}

// ── a database that does not open ───────────────────────────────────────────

/// What the user chose when the database did not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Show,
    MoveAside,
    Quit,
}

#[cfg(target_os = "macos")]
const SHOW_LABEL: &str = "Show in Finder";
#[cfg(not(target_os = "macos"))]
const SHOW_LABEL: &str = "Show Folder";
const MOVE_ASIDE_LABEL: &str = "Move Aside and Start Fresh";
const QUIT_LABEL: &str = "Quit";

/// The database, migrated. When it does not open (a migration fails, a
/// newer Ranch wrote it, the file is damaged), the user is told why and
/// where the file is, and chooses: show it, set it aside and start with an
/// empty one, or quit. The app never stops without a word.
pub fn open_database(app_data_dir: &Path, log: Option<&Path>) -> Db {
    loop {
        let error = match tauri::async_runtime::block_on(Db::init(app_data_dir)) {
            Ok(db) => return db,
            Err(e) => e,
        };
        tracing::error!(error = %error, "the database did not open");
        let db_path = Db::path(app_data_dir);
        match ask(&database_message(&error, &db_path, log)) {
            Some(Choice::Show) => {
                let target = if db_path.exists() {
                    db_path.as_path()
                } else {
                    app_data_dir
                };
                if let Err(e) = tauri_plugin_opener::reveal_item_in_dir(target) {
                    tracing::warn!(error = %e, "could not show the database");
                }
                std::process::exit(1);
            }
            Some(Choice::MoveAside) => match move_aside(&db_path) {
                Ok(kept) => {
                    tracing::warn!(?kept, "database set aside; starting with an empty one");
                }
                Err(e) => {
                    tracing::error!(error = %e, "could not set the database aside");
                    std::process::exit(1);
                }
            },
            Some(Choice::Quit) => std::process::exit(1),
            None => {
                eprintln!(
                    "Ranch could not open its database ({}): {error}",
                    db_path.display()
                );
                std::process::exit(1);
            }
        }
    }
}

fn database_message(error: &RanchError, db_path: &Path, log: Option<&Path>) -> String {
    let mut message = format!("{error}\n\nDatabase: {}\n", db_path.display());
    if let Some(log) = log {
        message.push_str(&format!("Log: {}\n", log.display()));
    }
    message.push_str(
        "\nBefore it updates the database, Ranch copies it beside itself (ranch.db.bak-schema-…).\n\n",
    );
    message.push_str(&format!(
        "{SHOW_LABEL}: quit and show the file.\n{MOVE_ASIDE_LABEL}: rename the file (it is kept) and start with no connections.\n{QUIT_LABEL}: change nothing."
    ));
    message
}

/// Rename the database and its journal files beside themselves, the
/// journal names following the database's so the copy still opens.
fn move_aside(db_path: &Path) -> std::io::Result<PathBuf> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let mut kept = db_path.as_os_str().to_owned();
    kept.push(format!(".set-aside-{stamp}"));
    let kept = PathBuf::from(kept);
    for suffix in ["", "-wal", "-shm"] {
        let mut from = db_path.as_os_str().to_owned();
        from.push(suffix);
        let from = PathBuf::from(from);
        if !from.exists() {
            continue;
        }
        let mut to = kept.as_os_str().to_owned();
        to.push(suffix);
        std::fs::rename(&from, PathBuf::from(to))?;
    }
    Ok(kept)
}

const DIALOG_TITLE: &str = "Ranch cannot open its database";

/// A native alert with the three choices, through the OS's own tool (no
/// window of the app exists yet). `None` when none could be shown.
#[cfg(target_os = "macos")]
fn ask(message: &str) -> Option<Choice> {
    // The texts go in as arguments: nothing in them is read as script.
    let script = format!(
        "on run argv\ndisplay alert (item 1 of argv) message (item 2 of argv) as critical buttons {{\"{QUIT_LABEL}\", \"{MOVE_ASIDE_LABEL}\", \"{SHOW_LABEL}\"}} default button \"{SHOW_LABEL}\" cancel button \"{QUIT_LABEL}\"\nend run"
    );
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .arg(DIALOG_TITLE)
        .arg(message)
        .output()
        .ok()?;
    let answer = String::from_utf8_lossy(&output.stdout);
    Some(if answer.contains(SHOW_LABEL) {
        Choice::Show
    } else if answer.contains(MOVE_ASIDE_LABEL) {
        Choice::MoveAside
    } else {
        Choice::Quit
    })
}

#[cfg(target_os = "linux")]
fn ask(message: &str) -> Option<Choice> {
    if let Ok(output) = Command::new("zenity")
        .args([
            "--question",
            "--no-markup",
            "--title",
            DIALOG_TITLE,
            "--text",
        ])
        .arg(message)
        .args([
            "--ok-label",
            SHOW_LABEL,
            "--cancel-label",
            QUIT_LABEL,
            "--extra-button",
            MOVE_ASIDE_LABEL,
        ])
        .output()
    {
        return Some(if output.status.success() {
            Choice::Show
        } else if String::from_utf8_lossy(&output.stdout).contains(MOVE_ASIDE_LABEL) {
            Choice::MoveAside
        } else {
            Choice::Quit
        });
    }
    let status = Command::new("kdialog")
        .args(["--title", DIALOG_TITLE, "--warningyesnocancel"])
        .arg(message)
        .args([
            "--yes-label",
            SHOW_LABEL,
            "--no-label",
            MOVE_ASIDE_LABEL,
            "--cancel-label",
            QUIT_LABEL,
        ])
        .status()
        .ok()?;
    Some(match status.code() {
        Some(0) => Choice::Show,
        Some(1) => Choice::MoveAside,
        _ => Choice::Quit,
    })
}

#[cfg(target_os = "windows")]
fn ask(message: &str) -> Option<Choice> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // The text goes in through the environment: nothing in it is read as
    // script.
    let text =
        format!("{message}\n\nYes: {SHOW_LABEL}. No: {MOVE_ASIDE_LABEL}. Cancel: {QUIT_LABEL}.");
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show($env:RANCH_DIALOG_TEXT, $env:RANCH_DIALOG_TITLE, 'YesNoCancel', 'Error')",
        ])
        .env("RANCH_DIALOG_TEXT", text)
        .env("RANCH_DIALOG_TITLE", DIALOG_TITLE)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let answer = String::from_utf8_lossy(&output.stdout);
    Some(match answer.trim() {
        "Yes" => Choice::Show,
        "No" => Choice::MoveAside,
        _ => Choice::Quit,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn ask(_message: &str) -> Option<Choice> {
    None
}

// ── a key of the seals that is gone ─────────────────────────────────────────

/// The safety settings as `ranch-mcp` reads a value the app did not seal:
/// AI off, every write asks (the default mode), agents read-only by
/// default, names only shared, default masks.
const STRICT_SETTINGS: [(SafetySetting, Option<&str>); 5] = [
    (SafetySetting::AiEnabled, Some("false")),
    (SafetySetting::ApprovalMode, None),
    (SafetySetting::ReadonlyDefault, Some("true")),
    (SafetySetting::ShareMode, Some("names")),
    (SafetySetting::SharePatterns, None),
];

/// The key of the seals is gone (its keychain entry deleted, another
/// keychain) while the database holds seals made with it. Sealing the
/// database as it is would vouch for whatever another process wrote in
/// it: start from the strictest values instead, every connection locked
/// and read-only for agents, and seal those.
pub async fn restrict_after_key_loss(state: &AppState) -> Result<(), RanchError> {
    for (setting, value) in STRICT_SETTINGS {
        match value {
            Some(value) => state.settings.set(setting.key(), value).await?,
            None => state.settings.remove(setting.key()).await?,
        }
    }
    state.repository.restrict_all().await
}

pub const KEY_LOSS_TITLE: &str = "Ranch reset its agent safety settings";
pub const KEY_LOSS_BODY: &str = "The key that protects them was missing from the keychain. AI features are off, and every connection is locked and read-only for agents until you change it.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_says_where_the_file_is_and_what_each_choice_does() {
        let error = RanchError::InvalidInput {
            reason: "boom".into(),
        };
        let message = database_message(
            &error,
            Path::new("/data/ranch.db"),
            Some(Path::new("/logs/ranch.log")),
        );
        assert!(message.contains("boom"));
        assert!(message.contains("/data/ranch.db"));
        assert!(message.contains("/logs/ranch.log"));
        for label in [SHOW_LABEL, MOVE_ASIDE_LABEL, QUIT_LABEL] {
            assert!(message.contains(label), "{label}");
        }
    }

    #[test]
    fn a_database_set_aside_keeps_its_journal_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("ranch.db");
        std::fs::write(&db, b"db").unwrap();
        std::fs::write(dir.path().join("ranch.db-wal"), b"wal").unwrap();
        let kept = move_aside(&db).unwrap();
        assert!(!db.exists());
        assert_eq!(std::fs::read(&kept).unwrap(), b"db");
        let mut wal = kept.into_os_string();
        wal.push("-wal");
        assert_eq!(std::fs::read(PathBuf::from(wal)).unwrap(), b"wal");
    }
}
