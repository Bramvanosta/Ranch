# Keychain setup per OS

Ranch stores Redis connection credentials in your **OS keychain only**, never on disk in plain text. This document describes per-OS setup and troubleshooting.

The Ranch backend uses the [`keyring`](https://crates.io/crates/keyring) Rust crate as the cross-platform abstraction. Each connection's credential is stored under the service name `dev.ranch.app` with the connection's id as the username (account).

---

## macOS: Apple Keychain

✅ **Works out of the box.** No setup required.

When you save your first connection, macOS prompts you to grant Ranch access to the Keychain. After approval, subsequent reads/writes are silent (Keychain Access remembers the grant per app).

You can inspect or delete Ranch entries directly via **Keychain Access.app** → search for `dev.ranch.app`.

**The key of the agent seals** (it protects what agents may do: approvals, the agent safety settings, each connection's lock and where it points) has its own entry, service `dev.ranch.app.integrity`, account `integrity-key`. Earlier releases kept it as `ranch::integrity` under `dev.ranch.app`; Ranch copies it to the new entry and keeps the old one for one more release. The first time `ranch-mcp` (started by your agent client) reads the new entry, macOS may ask whether it may: choose **Always Allow**. Until then it uses the old entry. Do not delete either entry: without the key, Ranch resets agent safety to its strictest settings and every connection must be saved again before agents can use it.

---

## Windows: Credential Manager

✅ **Works out of the box.** No setup required.

Credentials appear in **Control Panel → User Accounts → Credential Manager → Windows Credentials** under "Generic Credentials" with the prefix `dev.ranch.app:`.

---

## Linux: libsecret / Secret Service

⚠️ **Requires a running secret-service provider.**

The Linux keyring crate uses the [Secret Service D-Bus API](https://specifications.freedesktop.org/secret-service/latest/), which means a credential-storage daemon must be running. The most common providers:

| Provider | Typical desktop environment |
|---|---|
| **GNOME Keyring** (`gnome-keyring-daemon`) | GNOME, Ubuntu, Fedora Workstation |
| **KWallet** (with `kwallet-pam` and the secret-service bridge) | KDE Plasma |
| **KeePassXC** (with secret-service integration enabled) | Any DE, as a third-party provider |

### Verify a provider is running

```bash
# Should list 'org.freedesktop.secrets':
busctl --user list | grep secret
```

If nothing shows, install and start one. Examples:

**Ubuntu / Debian** (GNOME Keyring is preinstalled on full GNOME installs):
```bash
sudo apt install gnome-keyring libsecret-1-0
# log out + back in, or:
gnome-keyring-daemon --start --components=secrets
```

**Arch / Fedora**:
```bash
sudo pacman -S gnome-keyring     # Arch
sudo dnf install gnome-keyring   # Fedora
```

**KeePassXC** (third-party, no DE assumption):
1. Install KeePassXC (`sudo apt install keepassxc` or equivalent).
2. Open it, **Tools → Settings → Secret Service Integration → Enable**.
3. Open a database; KeePassXC then serves any app that uses the keyring.

### Headless Linux (CI runners, servers, WSL2 without DE)

⚠️ **Known limitation.** Headless Linux environments without a graphical session typically don't have a running secret-service daemon. The desktop app has no fallback for this case: run headless, it reports `RanchError::KeychainUnavailable` on the first credential operation. It needs either a running gnome-keyring-daemon (with `--unlock`) or a graphical session.

For a server, use [Ranch server](server.md) instead: it keeps secrets in a file in its data directory, encrypted with `RANCH_SECRET_KEY`.

### Troubleshooting

If Ranch reports `KeychainUnavailable`, check in order:

1. `busctl --user list | grep secret`: is the D-Bus service registered?
2. `dbus-send --session --print-reply --dest=org.freedesktop.secrets /org/freedesktop/secrets org.freedesktop.DBus.Peer.Ping`: does it respond?
3. Check the daemon's status (e.g. `systemctl --user status gnome-keyring-daemon`).
4. If using KeePassXC, ensure a database is open and unlocked.
