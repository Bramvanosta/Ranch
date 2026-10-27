//! Ask AI: hand a job, a cause or a connection to
//! the user's agent with the context ready.
//!
//! - **Claude Code** runs `claude "<prompt>"` in a terminal, in the code
//!   folder set for the group. The prompt never enters the command line: it
//!   is written to a private temporary file and read by the shell
//!   (`"$(cat …)"`), so nothing in a job's data can be run as a command.
//!   macOS drives Terminal through AppleScript (the first time, macOS asks
//!   whether Ranch may control Terminal); Linux starts `x-terminal-emulator`.
//! - **Claude Desktop / Cursor** cannot take a prompt from outside: the
//!   frontend copies it and this opens the app.
//! - **Copy prompt** is the frontend's alone.
//!
//! All UI-only and MCP-forbidden: an agent must never start processes on the
//! user's machine. Desktop-only.

use serde::{Deserialize, Serialize};
use specta::Type;

#[cfg(feature = "desktop")]
use crate::app_state::AppState;
use crate::core::errors::RanchError;
#[cfg(feature = "desktop")]
use crate::core::errors::RanchErrorDto;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentApp {
    ClaudeDesktop,
    Cursor,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LaunchClaudeCodeArgs {
    /// The group's code folder (Agents › Setup & safety).
    pub folder: String,
    pub prompt: String,
}

fn fail(reason: impl Into<String>) -> RanchError {
    RanchError::InvalidInput {
        reason: reason.into(),
    }
}

/// `'…'` for a POSIX shell: the only special character inside is `'`.
pub fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A string literal for AppleScript.
pub fn applescript_quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

/// The shell line that runs Claude Code on the prompt file, then deletes it.
pub fn claude_command(folder: &str, prompt_file: &str) -> String {
    format!(
        "cd -- {folder} && claude \"$(cat {file})\"; rm -f {file}",
        folder = shell_quote(folder),
        file = shell_quote(prompt_file),
    )
}

pub fn launch_claude_code_inner(args: &LaunchClaudeCodeArgs) -> Result<(), RanchError> {
    let folder = std::path::Path::new(args.folder.trim());
    if args.folder.trim().is_empty() || !folder.is_dir() {
        return Err(fail(format!(
            "The code folder {} does not exist. Set it in Agents › Setup & safety.",
            args.folder.trim()
        )));
    }
    if args.prompt.trim().is_empty() {
        return Err(fail("The prompt is empty."));
    }
    let file = write_private_prompt(&args.prompt)?;
    let command = claude_command(&folder.to_string_lossy(), &file.to_string_lossy());
    run_in_terminal(&command).inspect_err(|_| {
        let _ = std::fs::remove_file(&file);
    })
}

fn write_private_prompt(prompt: &str) -> Result<std::path::PathBuf, RanchError> {
    use std::io::Write;
    let path = std::env::temp_dir().join(format!("ranch-ask-{}.txt", uuid::Uuid::now_v7()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| fail(format!("Could not write the prompt: {e}")))?;
    file.write_all(prompt.as_bytes())
        .map_err(|e| fail(format!("Could not write the prompt: {e}")))?;
    Ok(path)
}

#[cfg(target_os = "macos")]
fn run_in_terminal(command: &str) -> Result<(), RanchError> {
    let script = format!(
        "tell application \"Terminal\"\nactivate\ndo script {}\nend tell",
        applescript_quote(command)
    );
    let status = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .status()
        .map_err(|e| fail(format!("Could not start Terminal: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(fail(
            "Terminal did not start Claude Code. Allow Ranch to control Terminal in System Settings › Privacy & Security › Automation, or copy the prompt.",
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn run_in_terminal(command: &str) -> Result<(), RanchError> {
    std::process::Command::new("x-terminal-emulator")
        .args(["-e", "sh", "-c", command])
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            fail(format!(
                "No terminal to start Claude Code in ({e}). Copy the prompt instead."
            ))
        })
}

pub fn open_agent_app_inner(app: AgentApp) -> Result<(), RanchError> {
    #[cfg(target_os = "macos")]
    {
        let name = match app {
            AgentApp::ClaudeDesktop => "Claude",
            AgentApp::Cursor => "Cursor",
        };
        let status = std::process::Command::new("open")
            .args(["-a", name])
            .status()
            .map_err(|e| fail(format!("Could not open {name}: {e}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(fail(format!(
                "{name} is not installed. The prompt is copied."
            )))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        match app {
            AgentApp::Cursor => std::process::Command::new("cursor")
                .spawn()
                .map(|_| ())
                .map_err(|e| {
                    fail(format!(
                        "Could not open Cursor ({e}). The prompt is copied."
                    ))
                }),
            AgentApp::ClaudeDesktop => Err(fail(
                "Claude Desktop is not available on this system. The prompt is copied.",
            )),
        }
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn launch_claude_code(args: LaunchClaudeCodeArgs) -> Result<(), RanchErrorDto> {
    launch_claude_code_inner(&args).map_err(Into::into)
}

#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn open_agent_app(app: AgentApp) -> Result<(), RanchErrorDto> {
    open_agent_app_inner(app).map_err(Into::into)
}

/// The masking policy applied to job data headed for a prompt: the same
/// masking an agent gets through MCP. JSON text in, JSON text out.
#[cfg(feature = "desktop")]
#[tauri::command]
#[specta::specta]
pub async fn share_for_agents(
    json: String,
    state: tauri::State<'_, AppState>,
) -> Result<String, RanchErrorDto> {
    let policy = crate::core::mcp::share::share_policy(state.db.pool()).await;
    Ok(crate::core::mcp::share::share_output(&policy, &json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_in_a_path_escapes_its_quotes() {
        assert_eq!(shell_quote("/a b"), "'/a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        let line = claude_command("/code/my app", "/tmp/ranch-ask-1.txt");
        assert_eq!(
            line,
            r#"cd -- '/code/my app' && claude "$(cat '/tmp/ranch-ask-1.txt')"; rm -f '/tmp/ranch-ask-1.txt'"#
        );
        assert_eq!(
            applescript_quote(r#"say "hi" \ bye"#),
            r#""say \"hi\" \\ bye""#
        );
    }

    #[test]
    fn a_missing_folder_is_explained_before_anything_runs() {
        let err = launch_claude_code_inner(&LaunchClaudeCodeArgs {
            folder: "/definitely/not/here".into(),
            prompt: "x".into(),
        })
        .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn the_prompt_file_is_private() {
        let path = write_private_prompt("$(rm -rf ~) `whoami`").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "$(rm -rf ~) `whoami`"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_file(path).unwrap();
    }
}
