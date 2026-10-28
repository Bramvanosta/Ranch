//! MCP installer module.
//!
//! Writes Ranch's MCP server entry into the config files of Claude
//! Desktop and Cursor non-destructively (preserves any pre-existing
//! `mcpServers` entries). Also exposes a manual-config snippet
//! generator for clients we don't auto-install.
//!
//! ## Config locations
//!
//!   - **Claude Desktop**:
//!     - macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
//!     - Windows: `%APPDATA%\Claude\claude_desktop_config.json`
//!     - Linux: `~/.config/Claude/claude_desktop_config.json`
//!   - **Cursor**:
//!     - All OS: `~/.cursor/mcp.json`
//!
//! ## JSON shape
//!
//! Both clients use:
//! ```json
//! {
//!   "mcpServers": {
//!     "ranch": { "command": "/path/to/ranch-mcp" }
//!   }
//! }
//! ```
//!
//! ## Non-destructive merge contract
//!
//! Reading existing JSON preserves every key. If the file doesn't exist,
//! it's created (with the parent dir). If the file exists but is
//! malformed, we error out (don't clobber the user's data).

use std::path::{Path, PathBuf};

use crate::domain::errors::RanchError;

/// The known MCP-aware clients Ranch can auto-install for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpClient {
    ClaudeDesktop,
    Cursor,
    /// JSON merge, like Cursor.
    Windsurf,
    /// Registered through the `claude` CLI.
    ClaudeCode,
    /// Registered through the `codex` CLI.
    Codex,
}

/// How a client is wired up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    /// Ranch merges `mcpServers.ranch` into the client's JSON config.
    JsonMerge,
    /// The client owns its config; Ranch asks the client's own CLI to
    /// register the server (`claude mcp add …`, `codex mcp add …`).
    ClientCli,
}

impl McpClient {
    pub const ALL: [McpClient; 5] = [
        Self::ClaudeDesktop,
        Self::ClaudeCode,
        Self::Cursor,
        Self::Windsurf,
        Self::Codex,
    ];

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Cursor => "Cursor",
            Self::Windsurf => "Windsurf",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    pub fn kind(&self) -> InstallerKind {
        match self {
            Self::ClaudeDesktop | Self::Cursor | Self::Windsurf => InstallerKind::JsonMerge,
            Self::ClaudeCode | Self::Codex => InstallerKind::ClientCli,
        }
    }

    /// Executable name of a `ClientCli` client.
    fn cli_name(&self) -> Option<&'static str> {
        match self {
            Self::ClaudeCode => Some("claude"),
            Self::Codex => Some("codex"),
            _ => None,
        }
    }

    /// Arguments that register `binary` as the `ranch` server. A fixed
    /// argument vector handed to `Command`, never a shell string, so a path
    /// with spaces or quotes cannot be reinterpreted.
    fn cli_args(&self, binary: &str) -> Option<Vec<String>> {
        let args: Vec<&str> = match self {
            Self::ClaudeCode => vec!["mcp", "add", "--scope", "user", "ranch", binary],
            Self::Codex => vec!["mcp", "add", "ranch", "--", binary],
            _ => return None,
        };
        Some(args.into_iter().map(str::to_string).collect())
    }
}

/// Per-client status returned to the frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpInstallerStatus {
    pub client: McpClient,
    pub config_path: String,
    /// `true` when the Ranch MCP entry is present in the client's config.
    pub installed: bool,
    /// `true` when the config dir / file exists (i.e. the client itself
    /// is installed). When `false`, install will create the dir + file.
    pub config_dir_exists: bool,
    /// Last-seen error encountered while reading the config (malformed
    /// JSON, permission denied, etc.). `None` for the happy path.
    pub error: Option<String>,
    /// How this client is installed.
    pub kind: InstallerKind,
    /// `ClientCli` only: was the client's CLI found? `None` for `JsonMerge`.
    pub cli_found: Option<bool>,
    /// `ClientCli` only: the exact command, to copy when the CLI was not
    /// found (or to run by hand).
    pub manual_command: Option<String>,
}

/// Resolve the absolute path to the client's MCP-config file. Returns
/// the canonical per-OS location even if the file doesn't exist yet.
pub fn config_path_for(client: McpClient) -> Result<PathBuf, RanchError> {
    let home = dirs_home()?;
    let path = match client {
        McpClient::ClaudeDesktop => {
            #[cfg(target_os = "macos")]
            {
                home.join("Library")
                    .join("Application Support")
                    .join("Claude")
                    .join("claude_desktop_config.json")
            }
            #[cfg(target_os = "linux")]
            {
                let xdg = std::env::var_os("XDG_CONFIG_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config"));
                xdg.join("Claude").join("claude_desktop_config.json")
            }
            #[cfg(target_os = "windows")]
            {
                let appdata = std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData").join("Roaming"));
                appdata.join("Claude").join("claude_desktop_config.json")
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
            {
                home.join(".config")
                    .join("Claude")
                    .join("claude_desktop_config.json")
            }
        }
        McpClient::Cursor => home.join(".cursor").join("mcp.json"),
        McpClient::Windsurf => home
            .join(".codeium")
            .join("windsurf")
            .join("mcp_config.json"),
        // `ClientCli` clients: the file Ranch READS to detect the entry. It
        // never writes there; the client's CLI does.
        McpClient::ClaudeCode => home.join(".claude.json"),
        McpClient::Codex => home.join(".codex").join("config.toml"),
    };
    Ok(path)
}

/// Find a client's CLI. A GUI app on macOS does not inherit the shell's
/// `PATH` (no `.zshrc`), so the usual install locations are searched too.
pub fn find_cli(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Ok(home) = dirs_home() {
        for rel in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".bun/bin",
            ".cargo/bin",
            ".volta/bin",
        ] {
            dirs.push(home.join(rel));
        }
    }
    for abs in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
        dirs.push(PathBuf::from(abs));
    }
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

fn quote_for_display(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:=".contains(c))
    {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// The command a human would type, for display / copy only, never executed
/// as a string.
fn manual_command_for(client: McpClient) -> Option<String> {
    let cli = client.cli_name()?;
    let binary = ranch_mcp_binary_path().ok()?;
    let args = client.cli_args(&binary.to_string_lossy())?;
    Some(
        std::iter::once(cli.to_string())
            .chain(args.iter().map(|a| quote_for_display(a)))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Is `ranch` registered in a `ClientCli` client's own config?
fn cli_client_has_ranch(client: McpClient, raw: &str) -> Result<bool, String> {
    match client {
        McpClient::ClaudeCode => serde_json::from_str::<serde_json::Value>(raw)
            .map(|v| v.get("mcpServers").and_then(|m| m.get("ranch")).is_some())
            .map_err(|e| format!("Existing config is not valid JSON: {e}")),
        // TOML table header; no TOML parser needed for a presence check.
        McpClient::Codex => Ok(raw
            .lines()
            .map(str::trim)
            .any(|l| l == "[mcp_servers.ranch]" || l == "[mcp_servers.\"ranch\"]")),
        _ => Ok(false),
    }
}

/// Read the user's home dir from `$HOME` / `%USERPROFILE%`.
fn dirs_home() -> Result<PathBuf, RanchError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| RanchError::InvalidInput {
                reason: "Cannot resolve $HOME".to_string(),
            })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .ok_or_else(|| RanchError::InvalidInput {
                reason: "Cannot resolve %USERPROFILE%".to_string(),
            })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err(RanchError::InvalidInput {
            reason: "Unsupported OS".to_string(),
        })
    }
}

/// Resolve the absolute path to the bundled `ranch-mcp` binary.
/// `std::env::current_exe()` gives the main app binary path; we derive
/// the sibling. In dev/test the `RANCH_MCP_BINARY` env var overrides.
pub fn ranch_mcp_binary_path() -> Result<PathBuf, RanchError> {
    // Debug builds only, like ranch-mcp's RANCH_MCP_DATA_DIR.
    #[cfg(debug_assertions)]
    if let Some(override_path) = std::env::var_os("RANCH_MCP_BINARY") {
        return Ok(PathBuf::from(override_path));
    }
    let current = std::env::current_exe().map_err(|e| RanchError::InvalidInput {
        reason: format!("Cannot resolve current exe: {e}"),
    })?;
    let parent = current.parent().ok_or_else(|| RanchError::InvalidInput {
        reason: "Current exe has no parent directory".to_string(),
    })?;
    let bin_name = if cfg!(target_os = "windows") {
        "ranch-mcp.exe"
    } else {
        "ranch-mcp"
    };
    Ok(parent.join(bin_name))
}

/// Read the existing config (if any) and report whether the `ranch`
/// MCP entry is present.
pub fn read_status_for(client: McpClient) -> McpInstallerStatus {
    let kind = client.kind();
    let cli_found = client.cli_name().map(|name| find_cli(name).is_some());
    let manual_command = manual_command_for(client);
    let base =
        |config_path: String, installed: bool, config_dir_exists: bool, error| McpInstallerStatus {
            client,
            config_path,
            installed,
            config_dir_exists,
            error,
            kind,
            cli_found,
            manual_command: manual_command.clone(),
        };

    let config_path = match config_path_for(client) {
        Ok(p) => p,
        Err(e) => return base(String::new(), false, false, Some(e.to_string())),
    };
    let path_str = config_path.to_string_lossy().to_string();
    let config_dir_exists = config_path.parent().map(|p| p.exists()).unwrap_or(false);

    if !config_path.exists() {
        return base(path_str, false, config_dir_exists, None);
    }
    let raw = match std::fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(e) => {
            return base(
                path_str,
                false,
                config_dir_exists,
                Some(format!("Cannot read config file: {e}")),
            )
        }
    };
    let parsed = match kind {
        InstallerKind::JsonMerge => serde_json::from_str::<serde_json::Value>(&raw)
            .map(|v| v.get("mcpServers").and_then(|m| m.get("ranch")).is_some())
            .map_err(|e| format!("Existing config is not valid JSON: {e}")),
        InstallerKind::ClientCli => cli_client_has_ranch(client, &raw),
    };
    match parsed {
        Ok(installed) => base(path_str, installed, config_dir_exists, None),
        Err(e) => base(path_str, false, config_dir_exists, Some(e)),
    }
}

/// Non-destructively merge the Ranch MCP entry into the client's
/// existing config. Creates parent dir + file if missing.
pub fn install_for(client: McpClient) -> Result<McpInstallerStatus, RanchError> {
    if client.kind() == InstallerKind::ClientCli {
        return install_via_cli(client);
    }
    let config_path = config_path_for(client)?;
    let binary = ranch_mcp_binary_path()?;

    // Read existing config, or start fresh.
    let mut config_value: serde_json::Value = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path).map_err(|e| RanchError::InvalidInput {
            reason: format!(
                "Cannot read existing config at {}: {e}",
                config_path.display()
            ),
        })?;
        if raw.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&raw).map_err(|e| RanchError::InvalidInput {
                reason: format!(
                    "Existing config at {} is not valid JSON: {e}. Refusing to overwrite.",
                    config_path.display()
                ),
            })?
        }
    } else {
        serde_json::json!({})
    };

    // Ensure `mcpServers` is an object map; merge in `ranch`.
    let servers = config_value
        .as_object_mut()
        .ok_or_else(|| RanchError::InvalidInput {
            reason: "Existing config root is not a JSON object".to_string(),
        })?
        .entry("mcpServers")
        .or_insert(serde_json::json!({}));
    let servers_map = servers
        .as_object_mut()
        .ok_or_else(|| RanchError::InvalidInput {
            reason: "Existing `mcpServers` is not a JSON object".to_string(),
        })?;
    servers_map.insert(
        "ranch".to_string(),
        serde_json::json!({
            "command": binary.to_string_lossy(),
        }),
    );

    // Write atomically: write to a temp file, then rename. Mirrors the
    // pattern Tauri uses for plist updates.
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| RanchError::InvalidInput {
            reason: format!("Cannot create config dir {}: {e}", parent.display()),
        })?;
    }
    let pretty =
        serde_json::to_string_pretty(&config_value).map_err(|e| RanchError::InvalidInput {
            reason: format!("Failed to serialize config: {e}"),
        })?;
    // Atomic write (temp file, then rename): a crash mid-write must not leave the
    // user's Claude / Cursor config truncated. A config that is a symlink
    // (dotfiles repos) is written where it points, not replaced by a file.
    // The temp file is created new (never through a link planted at its
    // name) with the original's permissions: these configs hold other
    // servers' API keys, a 0600 file must not come back 0644.
    let config_path = std::fs::canonicalize(&config_path).unwrap_or(config_path);
    let tmp_path = config_path.with_extension("json.ranch-tmp");
    let _ = std::fs::remove_file(&tmp_path);
    write_new_file(&tmp_path, pretty.as_bytes(), &config_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        RanchError::InvalidInput {
            reason: format!("Cannot write config at {}: {e}", tmp_path.display()),
        }
    })?;
    std::fs::rename(&tmp_path, &config_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        RanchError::InvalidInput {
            reason: format!("Cannot replace config at {}: {e}", config_path.display()),
        }
    })?;

    Ok(read_status_for(client))
}

/// Create `path` (failing if anything is already there, a symlink included)
/// with `contents`, and the permissions of `like` when it exists (0600 for a
/// new config).
fn write_new_file(path: &Path, contents: &[u8], like: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mode = std::fs::metadata(like)
            .map(|m| m.permissions().mode() & 0o777)
            .unwrap_or(0o600);
        options.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = like;
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

/// Register through the client's own CLI. The client owns its
/// config format (Claude Code's `~/.claude.json` holds much more than MCP
/// servers; Codex uses TOML): letting its CLI write is the non-destructive
/// option.
fn install_via_cli(client: McpClient) -> Result<McpInstallerStatus, RanchError> {
    let cli_name = client.cli_name().ok_or_else(|| RanchError::InvalidInput {
        reason: format!("{} is not installed through a CLI", client.display_name()),
    })?;
    let cli = find_cli(cli_name).ok_or_else(|| RanchError::InvalidInput {
        reason: format!(
            "The `{cli_name}` command was not found. Install {} first, or run the manual command.",
            client.display_name()
        ),
    })?;
    let binary = ranch_mcp_binary_path()?;
    let args = client
        .cli_args(&binary.to_string_lossy())
        .unwrap_or_default();

    // Re-registering must update the path (the app may have moved): remove
    // any previous entry first. A failure here just means there was none.
    if read_status_for(client).installed {
        let remove: Vec<&str> = match client {
            McpClient::ClaudeCode => vec!["mcp", "remove", "--scope", "user", "ranch"],
            _ => vec!["mcp", "remove", "ranch"],
        };
        let _ = std::process::Command::new(&cli).args(remove).output();
    }

    let output = std::process::Command::new(&cli)
        .args(&args)
        .output()
        .map_err(|e| RanchError::InvalidInput {
            reason: format!("Could not run `{cli_name}`: {e}"),
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if stderr.trim().is_empty() {
            stdout
        } else {
            stderr
        };
        return Err(RanchError::InvalidInput {
            reason: format!("`{cli_name} mcp add` failed: {}", detail.trim()),
        });
    }
    Ok(read_status_for(client))
}

/// manual-config snippet for clients we don't auto-install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpManualSnippet {
    pub binary_path: String,
    pub json_snippet: String,
}

pub fn manual_snippet() -> Result<McpManualSnippet, RanchError> {
    let binary = ranch_mcp_binary_path()?;
    let snippet = serde_json::json!({
        "mcpServers": {
            "ranch": {
                "command": binary.to_string_lossy(),
            }
        }
    });
    Ok(McpManualSnippet {
        binary_path: binary.to_string_lossy().to_string(),
        json_snippet: serde_json::to_string_pretty(&snippet).map_err(|e| {
            RanchError::InvalidInput {
                reason: format!("Failed to serialize manual snippet: {e}"),
            }
        })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Tests mutate process-wide env (`HOME`, `USERPROFILE`,
    /// `RANCH_MCP_BINARY`). Cargo runs them in parallel by default;
    /// serialize via this mutex to keep the world consistent.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Hold the `ENV_LOCK` guard for the test's lifetime AND own the
    /// `TempDir`. Dropping in order (guard last) keeps env consistent
    /// until the test finishes asserting.
    struct TestHome {
        _tmp: TempDir,
        _guard: std::sync::MutexGuard<'static, ()>,
        config_path: std::path::PathBuf,
        /// Some tests point `PATH` at a fake CLI; put it back afterwards
        /// so nothing else in this test process inherits it.
        original_path: Option<std::ffi::OsString>,
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            // SAFETY: still holding ENV_LOCK (fields drop after this runs).
            unsafe {
                match &self.original_path {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    fn with_tmp_home() -> TestHome {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: serialized via ENV_LOCK above.
        unsafe {
            std::env::set_var("HOME", tmp.path());
            std::env::set_var("USERPROFILE", tmp.path());
            // CI runners set these; the config path must come from the temp home.
            std::env::set_var("XDG_CONFIG_HOME", tmp.path().join(".config"));
            std::env::set_var("APPDATA", tmp.path().join("AppData").join("Roaming"));
            std::env::set_var("RANCH_MCP_BINARY", tmp.path().join("ranch-mcp"));
        }
        let config_path = config_path_for(McpClient::ClaudeDesktop).unwrap();
        TestHome {
            _tmp: tmp,
            _guard: guard,
            config_path,
            original_path: std::env::var_os("PATH"),
        }
    }

    #[test]
    fn install_writes_fresh_config_when_none_exists() {
        let h = with_tmp_home();
        let config_path = h.config_path.clone();
        assert!(!config_path.exists());
        let status = install_for(McpClient::ClaudeDesktop).unwrap();
        assert!(status.installed);
        assert!(config_path.exists());
        let raw = std::fs::read_to_string(&config_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(v["mcpServers"]["ranch"]["command"].is_string());
    }

    #[test]
    fn install_preserves_existing_servers() {
        let h = with_tmp_home();
        let config_path = h.config_path.clone();
        // Seed with an existing config that has another server.
        let existing = serde_json::json!({
            "mcpServers": {
                "github": { "command": "/usr/local/bin/github-mcp" }
            },
            "otherTopLevel": 42
        });
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, existing.to_string()).unwrap();

        install_for(McpClient::ClaudeDesktop).unwrap();

        let raw = std::fs::read_to_string(&config_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        // Pre-existing server preserved.
        assert_eq!(
            v["mcpServers"]["github"]["command"].as_str().unwrap(),
            "/usr/local/bin/github-mcp"
        );
        // Pre-existing top-level key preserved.
        assert_eq!(v["otherTopLevel"].as_i64().unwrap(), 42);
        // Ranch server added.
        assert!(v["mcpServers"]["ranch"]["command"].is_string());
    }

    #[test]
    fn install_updates_existing_ranch_entry() {
        let h = with_tmp_home();
        let config_path = h.config_path.clone();
        let existing = serde_json::json!({
            "mcpServers": {
                "ranch": { "command": "/stale/path/ranch-mcp" }
            }
        });
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, existing.to_string()).unwrap();

        install_for(McpClient::ClaudeDesktop).unwrap();
        let raw = std::fs::read_to_string(&config_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let cmd = v["mcpServers"]["ranch"]["command"].as_str().unwrap();
        assert!(
            !cmd.contains("stale"),
            "should overwrite stale path; got {cmd}"
        );
    }

    #[test]
    fn install_refuses_to_overwrite_malformed_json() {
        let h = with_tmp_home();
        let config_path = h.config_path.clone();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, "{ this is not json }").unwrap();
        let err = install_for(McpClient::ClaudeDesktop).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not valid JSON") && msg.contains("Refusing"),
            "expected refuse-to-overwrite message; got: {msg}"
        );
    }

    #[test]
    fn read_status_reports_installed_when_ranch_entry_present() {
        let h = with_tmp_home();
        let config_path = h.config_path.clone();
        let existing = serde_json::json!({
            "mcpServers": { "ranch": { "command": "/x/ranch-mcp" } }
        });
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(&config_path, existing.to_string()).unwrap();
        let s = read_status_for(McpClient::ClaudeDesktop);
        assert!(s.installed);
        assert!(s.config_dir_exists);
        assert!(s.error.is_none());
    }

    #[test]
    fn manual_snippet_contains_ranch_block() {
        let _h = with_tmp_home();
        let snip = manual_snippet().unwrap();
        let v: serde_json::Value = serde_json::from_str(&snip.json_snippet).unwrap();
        assert!(v["mcpServers"]["ranch"]["command"].is_string());
        assert!(!snip.binary_path.is_empty());
    }

    // ── Other clients (CLI-driven installs) ───────────────────────────────

    #[test]
    fn windsurf_is_a_json_merge_at_its_own_path() {
        let _h = with_tmp_home();
        let path = config_path_for(McpClient::Windsurf).unwrap();
        assert!(path.ends_with(".codeium/windsurf/mcp_config.json"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"mcpServers":{"github":{"command":"gh-mcp"}}}"#).unwrap();

        let status = install_for(McpClient::Windsurf).unwrap();
        assert!(status.installed);
        assert_eq!(status.kind, InstallerKind::JsonMerge);
        assert_eq!(status.cli_found, None);
        assert_eq!(status.manual_command, None);

        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["github"]["command"], "gh-mcp");
        assert!(v["mcpServers"]["ranch"]["command"].is_string());
        // The temp file of the atomic write is gone.
        assert!(!path.with_extension("json.ranch-tmp").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_private_config_stays_private_and_a_symlinked_one_stays_a_symlink() {
        use std::os::unix::fs::PermissionsExt;
        let _h = with_tmp_home();
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let path = config_path_for(McpClient::Windsurf).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // The real file lives elsewhere (a dotfiles repo), with API keys in it.
        let real = home.join("dotfiles-mcp.json");
        std::fs::write(&real, r#"{"mcpServers":{"x":{"env":{"KEY":"s3cret"}}}}"#).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();

        install_for(McpClient::Windsurf).unwrap();

        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&real).unwrap()).unwrap();
        assert!(v["mcpServers"]["ranch"]["command"].is_string());
        assert_eq!(v["mcpServers"]["x"]["env"]["KEY"], "s3cret");
        let mode = std::fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_malformed_config_is_never_replaced_and_leaves_no_temp_file() {
        let _h = with_tmp_home();
        let path = config_path_for(McpClient::Windsurf).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ nope").unwrap();
        assert!(install_for(McpClient::Windsurf).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ nope");
        assert!(!path.with_extension("json.ranch-tmp").exists());
    }

    #[test]
    fn cli_client_without_its_cli_reports_not_found_and_offers_the_command() {
        let h = with_tmp_home();
        // An empty PATH, and a HOME with none of the usual install dirs.
        unsafe { std::env::set_var("PATH", h._tmp.path().join("empty-bin")) };
        if find_cli("claude").is_some() {
            // A system-wide install (/opt/homebrew/bin, /usr/local/bin) is
            // outside what a test can hide; the detection path is then
            // exercised by the fake-CLI test below.
            return;
        }
        let status = read_status_for(McpClient::ClaudeCode);
        assert_eq!(status.kind, InstallerKind::ClientCli);
        assert_eq!(status.cli_found, Some(false));
        assert!(!status.installed);
        let cmd = status.manual_command.unwrap();
        assert!(
            cmd.starts_with("claude mcp add --scope user ranch "),
            "{cmd}"
        );

        let err = install_for(McpClient::ClaudeCode).unwrap_err().to_string();
        assert!(err.contains("`claude` command was not found"), "{err}");
    }

    #[test]
    fn detection_reads_the_clients_own_config_format() {
        assert!(cli_client_has_ranch(
            McpClient::ClaudeCode,
            r#"{"numStartups":3,"mcpServers":{"ranch":{"command":"/x/ranch-mcp"}}}"#
        )
        .unwrap());
        assert!(!cli_client_has_ranch(McpClient::ClaudeCode, r#"{"mcpServers":{}}"#).unwrap());
        assert!(cli_client_has_ranch(McpClient::ClaudeCode, "{ nope").is_err());

        let toml = "model = \"o3\"\n\n[mcp_servers.ranch]\ncommand = \"/x/ranch-mcp\"\n";
        assert!(cli_client_has_ranch(McpClient::Codex, toml).unwrap());
        assert!(!cli_client_has_ranch(McpClient::Codex, "[mcp_servers.other]\n").unwrap());
    }

    #[test]
    fn the_displayed_command_quotes_awkward_paths() {
        assert_eq!(
            quote_for_display("/usr/local/bin/ranch-mcp"),
            "/usr/local/bin/ranch-mcp"
        );
        assert_eq!(
            quote_for_display("/Applications/My Apps/Ranch.app/ranch-mcp"),
            "'/Applications/My Apps/Ranch.app/ranch-mcp'"
        );
        assert_eq!(quote_for_display("it's"), "'it'\\''s'");
    }

    /// A fake `claude` on PATH: records its argv, and on `mcp add` writes the
    /// entry the real CLI would write, so the round-trip (find → run with a
    /// fixed argv → detect) is exercised without the real client.
    #[cfg(unix)]
    #[test]
    fn claude_code_is_installed_through_its_cli_with_a_fixed_argv() {
        use std::os::unix::fs::PermissionsExt;
        let h = with_tmp_home();
        let bin = h._tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = h._tmp.path().join("argv.log");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{log}'\necho '---' >> '{log}'\nif [ \"$2\" = add ]; then printf '{{\"mcpServers\":{{\"ranch\":{{\"command\":\"%s\"}}}}}}' \"$6\" > \"$HOME/.claude.json\"; fi\n",
            log = log.display()
        );
        let cli = bin.join("claude");
        std::fs::write(&cli, script).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A binary path with a space: must reach the CLI as ONE argument.
        let ranch_mcp = h._tmp.path().join("My Apps").join("ranch-mcp");
        unsafe {
            std::env::set_var("PATH", &bin);
            std::env::set_var("RANCH_MCP_BINARY", &ranch_mcp);
        }

        assert_eq!(read_status_for(McpClient::ClaudeCode).cli_found, Some(true));
        let status = install_for(McpClient::ClaudeCode).unwrap();
        assert!(status.installed);

        let argv = std::fs::read_to_string(&log).unwrap();
        let expected = format!(
            "mcp\nadd\n--scope\nuser\nranch\n{}\n---\n",
            ranch_mcp.display()
        );
        assert_eq!(
            argv, expected,
            "exactly one call, path kept as a single argument"
        );

        // Re-install: the stale entry is removed first, then added again.
        install_for(McpClient::ClaudeCode).unwrap();
        let argv = std::fs::read_to_string(&log).unwrap();
        assert!(
            argv.contains("mcp\nremove\n--scope\nuser\nranch\n---\n"),
            "{argv}"
        );
    }
}
