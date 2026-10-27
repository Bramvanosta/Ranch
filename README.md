<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset=".github/assets/ranch-lockup-dark.png">
    <img src=".github/assets/ranch-lockup-light.png" alt="ranch" width="280">
  </picture>
</p>

<p align="center">
  A desktop client for your BullMQ queues: see what is waiting, running and failing,<br>
  drill into a job, retry or clean up — and let your AI agents do the same, under your control.
</p>

<p align="center">
  <a href="#installation"><b>Download</b></a> ·
  <a href="#features"><b>Features</b></a> ·
  <a href="#ai-agents-mcp"><b>AI agents</b></a> ·
  <a href="#ranch-server-docker"><b>Ranch server</b></a> ·
  <a href="CHANGELOG.md"><b>Changelog</b></a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-universal-17181c?logo=apple&amp;logoColor=white" alt="macOS universal">
  <img src="https://img.shields.io/badge/Linux-deb%20%C2%B7%20rpm%20%C2%B7%20AppImage-17181c?logo=linux&amp;logoColor=white" alt="Linux: deb, rpm, AppImage">
  <img src="https://img.shields.io/badge/Tauri-v2-17181c?logo=tauri&amp;logoColor=white" alt="Tauri v2">
  <img src="https://img.shields.io/badge/MCP-server%20included-17181c" alt="MCP server included">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-f28c38" alt="MIT license"></a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset=".github/assets/screenshot-light.png">
    <img src=".github/assets/screenshot-dark.png" alt="Ranch on a production connection: counters, what needs attention with an agent's diagnosis, and live events" width="900">
  </picture>
</p>

## Contents

- [Why Ranch](#why-ranch)
- [Installation](#installation)
- [Features](#features)
- [AI agents (MCP)](#ai-agents-mcp)
- [Ranch server (Docker)](#ranch-server-docker)
- [Development](#development)
- [Tech stack](#tech-stack)
- [Contributing](#contributing)
- [License](#license)

## Why Ranch

- **A desktop app first.** It runs on your machine, talks to Redis directly — locally, over TLS or through an SSH tunnel — and keeps your secrets in the OS keychain. Where a desktop app does not fit, the same UI runs as a [Docker image](#ranch-server-docker).
- **Every connection in one window.** Projects, environments and Redis instances side by side, with what needs you first.
- **Safe on production.** Production connections are read-only until you unlock them, every destructive action shows its count before it runs, and every write lands in an audit log.
- **Writes are BullMQ-exact.** Retry, promote, add, drain, clean and schedulers go through Lua scripts ported from BullMQ's own, proven in CI by real BullMQ **5.x and 6.x** workers processing the result.
- **AI agents, on a leash.** The built-in MCP server gives your agents the same queues: reads are free, writes wait for your approval in Ranch.

## Installation

Download the latest build from the [Releases](../../releases/latest) page.

### macOS

Open the `.dmg` (one signed and notarized universal app, for Apple Silicon and Intel) and drag Ranch into Applications.

### Linux

```bash
sudo apt install ./Ranch_*_amd64.deb      # Debian, Ubuntu
sudo dnf install ./Ranch-*.x86_64.rpm     # Fedora, RHEL
chmod +x Ranch_*.AppImage && ./Ranch_*.AppImage
```

> [!IMPORTANT]
> Ranch stores Redis and SSH passwords in the OS keychain. On Linux it needs a running Secret Service provider (GNOME Keyring, KWallet or KeePassXC), or saving a connection fails. See [docs/keychain-setup.md](docs/keychain-setup.md).

### Windows

Not packaged yet. Build it from source (see [Development](#development)).

### Updates

The macOS app and the Linux AppImage update themselves: Ranch checks at start, shows the release notes and installs only when you click (Settings › Updates). A `.deb` or `.rpm` install is updated through your package manager.

## Features

### Connections

- **Groups, environments, connections.** Arrange them by project and environment, with a color per environment; drag and drop to reorder.
- **Every way to reach Redis.** `redis://` and `rediss://` URLs, TLS with a custom CA, SSH tunnels, secrets in the OS keychain.
- **A new connection in two steps.** Paste a URL or fill the fields, test, then see what Ranch found — latency, Redis version, the BullMQ queues, schedulers and workers — before you name and file it.
- **One connection per Redis, kept open.** Fast on a remote Redis: shared by every screen, reopened by itself after a drop, with reads running side by side.

### Triage

- **All connections at a glance.** One table per group: health and latency trend, queues, waiting, failed, workers, protection. An unreachable connection says why in plain words.
- **Needs attention.** Failing causes, stuck queues (paused too long, or jobs waiting with no worker) and offline connections across every connection, most urgent first, each with the action that fixes it.
- **Failures grouped by cause.** Reasons that only differ by their values fold into one group, with its count, job types and last 24 hours. Retry or remove a whole group at once.
- **⌘K reaches everything.** Queues, connections, pages, actions and jobs (`#1211`) from one palette.

### Queues and jobs

- **Per connection:** an overview (throughput, failure rate, timings from 5 minutes to 7 days), every queue with live counters, every failed job of every queue in one view, a timeline and a map of the queues.
- **Per queue:** jobs by state with server-side search, sort and paging across the whole queue; running jobs with their progress against the usual time of their type; schedulers (create, edit, pause, delete, with the next runs); flows; workers, and a warning when nothing consumes the queue.
- **Per job:** payload, options, progress, stack traces, return value, logs, and its flow as a graph. Retry, promote, remove, also in bulk and across queues.
- **Add Job** with a payload editor that validates as you type, and the matching `queue.add(...)` to copy.
- **Maintenance:** retry all failed, clean, drain — each with a count and a confirmation first.

### Safety

- **Production is locked** until you unlock it for the session.
- **An audit log** of every write — yours and your agents' — filterable and exportable as CSV or JSON.
- **Light and dark themes**, both checked for WCAG AA text contrast.

## AI agents (MCP)

Ranch ships an MCP server, `ranch-mcp`, that works with the connections you set up in the app.

- **One-click setup** for Claude Desktop, Claude Code, Cursor, Codex and Windsurf (Agents › Setup & safety).
- **Reads are free, writes need you.** An agent's write opens an approval in Ranch: what it would do, to how many jobs, the exact call. Approve once or deny.
- **Per-connection access:** read-only, ask, or free. A global switch turns every AI feature off.
- **Job data you choose to share:** sensitive fields masked by default (email, token, password, card numbers…), or names and errors only.
- **Notes on failures:** an agent that found why jobs fail attaches its diagnosis to the cause, shown on the Failed tab.
- **Ask AI** on a job, a cause or a connection hands the problem to your agent with the context ready.

> *“Why are jobs failing in the emails queue on production since this morning?”*
>
> *“Retry the failed stripe-webhooks jobs that timed out, and tell me how many.”*

The full list of tools is in [docs/mcp-tools.md](docs/mcp-tools.md).

## Ranch server (Docker)

The same UI, served over HTTP by the same Rust core, for the places a desktop app does not go: next to a staging Redis, in a compose stack, in a cluster.

```bash
docker run -d --name ranch -p 8080:8080 -v ranch-data:/data \
  -e RANCH_PASSWORD='choose-one' -e RANCH_SECRET_KEY='another-one' \
  ghcr.io/bramvanosta/ranch:latest
```

Open http://localhost:8080 (user `ranch`). `:latest` is the newest release; pin a version (`:0.3.0`) to choose when you upgrade, or follow `:rc` (pre-releases) or `:main`.

To build the image from source instead:

```bash
docker build -t ranch .
docker run -d --name ranch -p 8080:8080 -v ranch-data:/data \
  -e RANCH_PASSWORD='choose-one' -e RANCH_SECRET_KEY='another-one' ranch
```

Or, with a demo Redis beside it:

```bash
RANCH_PASSWORD=choose-one docker compose -f docker/compose.yaml up --build
```

| Variable | Default | |
|---|---|---|
| `RANCH_PASSWORD` | — | Password of the single account (HTTP Basic). **Either this or OIDC is required**: without authentication the server refuses to start. |
| `RANCH_USERNAME` | `ranch` | Its user name. |
| `RANCH_OIDC_*` | — | Sign-in through your identity provider — see below. |
| `RANCH_SECRET_KEY` | unset | Encrypts the Redis / SSH passwords Ranch stores in `/data` (XChaCha20-Poly1305). Unset: stored in clear — the server says so at start-up. Keep it: a stored secret cannot be read with another key. |
| `RANCH_ALLOW_NO_AUTH` | unset | `1` = no password. Only on a network you trust end to end. |
| `RANCH_PORT` / `RANCH_BIND` | `8080` / `0.0.0.0` | |
| `RANCH_DATA_DIR` | `/data` | SQLite database (connections, groups, audit log, settings) and the secrets file. **Mount a volume.** A bind mount must be writable by uid 65532. |

<details>
<summary>Sign-in with OIDC (Pocket ID, Authentik, Keycloak…)</summary>

Register Ranch at the provider as a confidential client with the callback `https://<where Ranch lives>/auth/callback`, then:

```yaml
environment:
  RANCH_OIDC_ISSUER: https://id.example.com          # its /.well-known/openid-configuration is fetched
  RANCH_OIDC_CLIENT_ID: …
  RANCH_OIDC_CLIENT_SECRET: …
  RANCH_PUBLIC_URL: https://ranch.example.com         # the callback is <this>/auth/callback
  RANCH_SECRET_KEY: …                                 # also keeps sessions across restarts
  # RANCH_OIDC_ALLOWED_EMAILS: me@example.com,you@example.com
  # RANCH_OIDC_ALLOWED_GROUPS: admins                 # `groups` claim; either list matching is enough
```

| Variable | Default | |
|---|---|---|
| `RANCH_OIDC_ISSUER` | — | Turns OIDC on. The three below are then required, and a missing one is named at start-up. |
| `RANCH_OIDC_CLIENT_ID`, `RANCH_OIDC_CLIENT_SECRET` | — | The client registered at the provider. |
| `RANCH_PUBLIC_URL` | — | Where users reach Ranch. `https://` makes the cookies `Secure`. |
| `RANCH_OIDC_SCOPES` | `openid profile email` | `groups` is added when groups are filtered. |
| `RANCH_OIDC_ALLOWED_EMAILS`, `RANCH_OIDC_ALLOWED_GROUPS` | unset | Who may use Ranch. **Unset = whoever the provider lets into this client** (in Pocket ID, restrict the client to a user group). |
| `RANCH_SESSION_SECRET` | `RANCH_SECRET_KEY`, else random | Signs the session cookie. Random = everyone signs in again after a restart. |
| `RANCH_SESSION_TTL_HOURS` | `12` | How long a sign-in lasts. |

Authorization code + PKCE; the ID token is verified (signature against the provider's keys, asymmetric algorithms only, issuer, audience, expiry, nonce); the session is a signed, `HttpOnly`, `SameSite=Lax` cookie. When a session ends the page goes back to the provider and returns where it was. The status bar shows who is signed in, and *Sign out*. With OIDC on, `RANCH_PASSWORD` — if set — stays accepted as HTTP Basic: a break-glass for the day the provider is down, and a way in for scripts (`curl -u`). Leave it unset for OIDC only.

</details>

**Security model.** One shared account or your identity provider; credentials or the session cookie travel on every request, so **put TLS in front** (Caddy, Traefik, nginx…) before exposing it beyond localhost. The API refuses requests a foreign web page could forge (a required custom header), and everything except `/healthz` is behind the password. Ranch can pause queues and delete jobs: treat its URL like an admin console. Production connections are still locked by default, and every write is in the audit log.

**What the image does not do.** No MCP: agents need a human to approve their writes in the desktop app, next to the agent. No self-update (the desktop app has one): pull a newer tag. No live progress bar on bulk actions (they report when done). SSH tunnels work; a key file must be mounted into the container at the path you give Ranch.

The image is distroless (no shell), runs as non-root, and carries a `HEALTHCHECK` (`ranch-server --healthcheck`). Without Docker: `cargo build --release --no-default-features --features server --bin ranch-server` in `src-tauri/`, `pnpm build`, then run the binary with `RANCH_STATIC_DIR=dist`.

## Development

### Prerequisites

- [mise](https://mise.jdx.dev), which installs Rust and Node from `mise.toml`.
- [pnpm](https://pnpm.io), activated through `corepack`.
- The [Tauri system dependencies](https://v2.tauri.app/start/prerequisites/) for your platform.
- **Linux only:** a Secret Service provider, as above ([docs/keychain-setup.md](docs/keychain-setup.md)).

### Run the desktop app

```bash
mise install
corepack enable
pnpm install
pnpm tauri:dev
```

### Ranch web: the UI in a browser, without Redis

```bash
pnpm dev            # then open http://localhost:1420
```

Outside Tauri, a development build swaps the backend for an in-memory BullMQ world (`src/dev/webPreview.ts`): two groups, a local / staging / locked-production trio, an unreachable Redis, queues with jobs in every state, schedulers, workers, a flow, an audit trail, agent activity. Every screen, dialog, empty and error state can be looked at and clicked through: retry, remove, drain, add a job, edit a scheduler all act on the fake world. Nothing is saved; a reload starts over.

- `http://localhost:1420/?preview=empty` is a first launch (no connection, the wizard); `?preview=full` brings the demo world back.
- `ranchPreview.requestApproval()` in the browser console queues an agent write, to see the approval sheet.

The preview is loaded with a dynamic `import()` behind `import.meta.env.DEV`: it is not in production bundles.

### Checks

```bash
pnpm check          # format, Biome, types, unit tests
cd src-tauri && cargo test
```

The `parity_contract` integration test enforces UI/MCP parity for every command in `src-tauri/src/commands/registry.rs`: adding a Tauri command without an MCP equivalent fails CI.

<details>
<summary>Against a real Redis and real BullMQ workers</summary>

```bash
docker compose --profile redis --profile bullmq up -d        # Redis on :6390 + seeded queues
(cd .docker/bullmq-compat && npm ci)                          # bullmq@5 and bullmq@6
cd src-tauri && RANCH_LOCAL_REDIS=1 RANCH_REDIS_PORT=6390 RANCH_BULLMQ_COMPAT=1 cargo test
```

Without `RANCH_LOCAL_REDIS` the live suites skip.

</details>

<details>
<summary>Overflow check: no clipped or overlapping text</summary>

```bash
pnpm exec playwright install chromium   # once
pnpm check:overflow
```

Playwright opens Ranch web on every screen listed in `e2e/overflow/routes.ts`, in dark and light, at 1440×900 and 1100×700, and fails on text that does not fit: cut off or ellipsised (`clip`), running into its neighbour (`spill`), or leaving the window (`outside`). Each failure prints the screen, theme, size, overshoot in pixels and the text; the full results land in `e2e/.overflow-report.json`. It reuses a `pnpm dev` already running on port 1420, or starts one.

`e2e/overflow-allowlist.json` can hold known failures, but it is empty and should stay so: fix a failure instead of listing it. An entry that no longer matches anything fails the run ("stale allow-list entry").

Chromium only: it catches layout overflow as well as the macOS webview would. CI runs the check in the `mcr.microsoft.com/playwright:v<version>-noble` image of the `@playwright/test` version pinned in `package.json`: bumping the package bumps the image.

</details>

<details>
<summary>A local build to look at</summary>

```bash
pnpm tauri build --debug --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
open src-tauri/target/debug/bundle/macos/Ranch.app
```

`createUpdaterArtifacts` is switched off because signing the updater bundle needs the release key.

</details>

### Project structure

```
src/                  React front end
├── features/         one folder per area: home, connections, queues, jobs, mcp, …
├── components/       shared UI (data tables, sheets, the Ranch mark)
├── commands/         the command registry behind menus, shortcuts and ⌘K
├── layouts/          the window shell and sidebar
└── dev/              the in-memory preview for Ranch web
src-tauri/            Rust core
├── src/commands/     Tauri commands, mirrored one to one by MCP tools
├── src/core/         connections, BullMQ schema and Lua scripts, audit, keychain, MCP server
├── src/server/       the HTTP server behind the Docker image
└── src/bin/          ranch-mcp
```

[docs/architecture.md](docs/architecture.md) explains how the pieces fit; [docs/design.md](docs/design.md) is the design system.

## Tech stack

| | |
|---|---|
| **App shell** | Tauri v2 |
| **Front end** | React 19, TypeScript, Vite, Tailwind CSS v4, Lucide icons |
| **Core** | Rust, Tokio, `redis` with rustls, `rmcp` for the MCP server, Axum for Ranch server |
| **Tooling** | Biome, Vitest, Playwright, lefthook, mise |

## Contributing

Issues and pull requests are welcome. Before opening a pull request, run `pnpm check` and `cargo test` in `src-tauri/`, and add an entry under *Unreleased* in the [CHANGELOG](CHANGELOG.md).

## License

[MIT](LICENSE)
