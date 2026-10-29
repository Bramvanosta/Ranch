# CI and releases

Everything runs on GitHub Actions, from the workflows in `.github/workflows/`:

| Workflow | When | What |
|---|---|---|
| `ci.yml` | push to `main`, pull requests | frontend (Biome, types, unit tests, no credentials in SQL migrations); on pull requests: no shipped migration changed; overflow check; Rust with the `server` feature; Rust with the `desktop` feature; on pull requests: the server image, built and smoke-tested |
| `release.yml` | version tags | checks the tag is a commit of `main` whose CI passed, creates the GitHub release as a draft, builds the Linux bundles and the macOS app in parallel and attaches them, then publishes the release, the updates and the Homebrew casks |
| `image.yml` | push to `main`, version tags | builds the server image for amd64 and arm64, **smoke-tests each**, pushes them to `ghcr.io/bramvanosta/ranch` (a tag: only from a commit of `main` whose CI passed) |

The release and the image use the workflow's own `GITHUB_TOKEN`: there is no token to create or rotate.

## CI

- **frontend**: `bash scripts/check-no-creds-in-sql.sh`, `pnpm lint`, `pnpm typecheck`, `pnpm test` (Vitest and the `node --test` suites of `scripts/`). Node 24, pnpm through corepack at the version of `packageManager`.
- **migrations** (pull requests only): `scripts/check-migrations-unchanged.sh origin/<base>`. sqlx stores the checksum of every migration it applied and refuses to start when a file no longer matches: a migration that shipped never changes, not even a comment. New migrations are always fine; the newest one of the base branch may still change while no `v*` tag contains it.
- **overflow**: `pnpm check:overflow` in the `mcr.microsoft.com/playwright:v<version>-noble` image, which carries Chromium and its libraries. Its version is read from `@playwright/test` in `package.json`, which must stay pinned to an exact version: bumping the package bumps the image.
- **server**: `cargo fmt --check`, clippy and the library tests of every crate of the workspace (`--workspace`) with `--no-default-features --features server`: the binary of the Docker image, with no WebKit and nothing to install. A test here checks that every command is served over HTTP or refused as desktop-only, as the catalogue of operations says.
- **desktop**: clippy on every target of every crate and all the tests (`--workspace`), integration tests included, on Ubuntu with the Tauri system dependencies, against a seeded Redis service and real BullMQ 5 and 6 workers.
- **image** (pull requests only): `docker build`, then `scripts/smoke-test-image.sh`. On `main`, `image.yml` does the same and pushes.

**One Rust everywhere**: `rust-toolchain.toml` names the version (rustup reads it locally), and `mise.toml`, the workflows' `toolchain:` inputs and the Dockerfile's `rust:` image name the same one. Bump them together, in one commit: clippy gains lints with every release, so a bump can need fixes. The release builds use that version too and build exactly `Cargo.lock` (`--locked`), like CI. The Rust jobs build **without debug info** (`CARGO_PROFILE_DEV_DEBUG=0`): the desktop job links some thirty test binaries that each carry all of Tauri, and with debug info the linker can run out of memory. For the same reason those test binaries are linked three at a time. `Swatinem/rust-cache` keeps the dependencies between runs.

The **desktop** job runs the live suites too: a Redis service seeded with BullMQ queues (`.docker/bullmq-seed`), and the BullMQ 5 and 6 workers of `.docker/bullmq-compat` processing what Ranch's scripts wrote (`RANCH_LOCAL_REDIS`, `RANCH_BULLMQ_COMPAT`).

## Releasing

```bash
git tag v0.4.0-rc.1 && git push origin v0.4.0-rc.1    # pre-release
git tag v0.4.0      && git push origin v0.4.0         # release
```

Tag a commit of `main` once its CI has passed: both `release.yml` and `image.yml` check it (`scripts/check-release-commit.sh`) and stop otherwise. A tag pushed right after `main` waits for that CI, up to 25 minutes; past that, re-run the failed jobs once it is green.

The tag is the version: `scripts/set-version.mjs` writes it into `package.json`, `tauri.conf.json`, `Cargo.toml` and `Cargo.lock` inside each job. Nothing to bump by hand, nothing committed back. A tag that is not `vX.Y.Z` or `vX.Y.Z-<pre>` fails the jobs instead of publishing something odd.

| Pushed | Image tags | GitHub release |
|---|---|---|
| `main` | `:main` | - |
| `v0.4.0` | `:0.4.0` (what a deployment pins), `:latest` | release |
| `v0.4.0-rc.1` | `:0.4.0-rc.1`, `:rc` (`:latest` does not move) | **pre-release** |

`release.yml` runs in four jobs:

1. **release** checks the commit (above), then creates the release for the tag as a **draft** (a pre-release when the tag has a `-`): nobody sees it while it has no files. Its notes are the summary paragraph of the version's section of `CHANGELOG.md`, when there is one, followed by how to install. Re-running it updates the release instead of failing.
2. **linux** (`ubuntu-22.04`: an older glibc than today's distributions, so the AppImage runs on more of them) builds `.deb`, `.rpm` and `.AppImage`.
3. **macos** (`macos-14`) builds one universal app (Apple Silicon and Intel) in three steps: `tauri build --target universal-apple-darwin --no-bundle`, `lipo` to merge `ranch-mcp` (the universal build only merges the main binary), then `tauri bundle --features desktop --bundles app,dmg`.

4. **publish**, once both are done: makes the release public (and the latest one, for a stable version), publishes the updates (below), then updates the Homebrew tap. The tap comes last and its failure fails nothing: the release and the updates are out by then.

The two build jobs run side by side. Each attaches its files and a `SHA256SUMS-linux.txt` / `SHA256SUMS-macos.txt` with `gh release upload --clobber`, so re-running a job replaces what it had uploaded.

Keep the repository's *immutable releases* setting off: a re-run replaces files of a published release, and the update feed replaces its assets at every release.

Write the CHANGELOG section, with its summary paragraph, **before** tagging the release: the release page and the update dialog both take their text from the tag.

## Secrets

Settings → Secrets and variables → Actions → *Repository secrets*. All optional: without them the release is still built.

| Secret | Without it |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the bundles are built without updater archives and **no update is published** |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | the macOS app is ad-hoc signed and **Gatekeeper refuses it** once downloaded (see below) |
| `HOMEBREW_TAP_DEPLOY_KEY` | the Homebrew cask is not updated (see below) |

A secret that is not set reaches the job as an empty string; the macOS job removes the empty ones before building, so that Tauri does not try to sign with them.

### The Homebrew tap

[`Bramvanosta/homebrew-tap`](https://github.com/Bramvanosta/homebrew-tap) holds two casks of the universal DMG: `ranch` follows stable releases, `ranch@beta` follows every release, candidates and stable ones, so it is never behind `ranch`. The `publish` job writes the casks of the tag (`scripts/homebrew-cask.mjs`, with the DMG's checksum from `SHA256SUMS-macos.txt`; a cask never moves back to an older version) and pushes it to the tap with `HOMEBREW_TAP_DEPLOY_KEY`, the private half of an SSH deploy key that may write to the tap and to nothing else. A new key: `ssh-keygen -t ed25519 -N "" -f tap_key`, then `gh repo deploy-key add tap_key.pub --repo Bramvanosta/homebrew-tap --allow-write` and `gh secret set HOMEBREW_TAP_DEPLOY_KEY < tap_key`, and delete both files.

The casks download from this repository's releases, so `brew install` works once those are public.

### Signing and notarizing the macOS app

It needs a paid Apple Developer account:

1. developer.apple.com → Certificates → **Developer ID Application**; install it, then export it from Keychain Access as a `.p12` with a password.
2. Secrets: `APPLE_CERTIFICATE` = `base64 -i cert.p12`, `APPLE_CERTIFICATE_PASSWORD` = that password, `APPLE_SIGNING_IDENTITY` = `Developer ID Application: <Name> (<TEAMID>)`, `APPLE_TEAM_ID`, `APPLE_ID` = the account's e-mail, `APPLE_PASSWORD` = an **app-specific password** (account.apple.com → Sign-In and Security).
3. Tag again: the macOS job signs, notarizes and staples on its own (Tauri reads those variables); notarization adds a few minutes.

An unsigned build is quarantined once downloaded, and macOS answers *"Apple could not verify that Ranch.app is free of malware"*. For a build you trust: `xattr -dr com.apple.quarantine /Applications/Ranch.app`, or System Settings → Privacy & Security → *Open Anyway* right after the refused launch.

## Server image

`ghcr.io/bramvanosta/ranch`, linux/amd64 and linux/arm64. `image.yml` builds each architecture on a runner of its own (no emulation) with the tag's version written in, runs `scripts/smoke-test-image.sh` against it, and pushes that same image as `<tag>-<arch>`; the `manifest` job then publishes the tags as one multi-arch image of the two. The Dockerfile's base images are pinned by digest: bump one by changing its tag and taking the new digest from `docker buildx imagetools inspect`. It carries the `org.opencontainers.image.source` label of the repository, which links the package to it on GitHub.

The first push creates the package **private**: once, in the package's settings, set its visibility to *Public*.

`scripts/smoke-test-image.sh <image>` works on a laptop too. It talks to the container from inside its own network namespace (`--network container:…`), so it does not depend on how the caller networks its containers.

## Auto-updates

The desktop app updates itself from GitHub releases. GitHub's `releases/latest/download/…` skips pre-releases, and the app has a pre-release channel, so the manifests live on one fixed release, **`update-feed`**, whose assets are replaced at every publication:

```
releases/download/<tag>/Ranch.app.tar.gz (+ .sig)        the universal macOS archive
releases/download/<tag>/Ranch_<version>_amd64.AppImage (+ .sig)
releases/download/<tag>/platform-<platform>.json          { signature, url }, one per platform
releases/download/update-feed/latest.json                 newest STABLE version  ← stable builds
releases/download/update-feed/rc.json                     newest tag, RC or not   ← pre-release builds
```

(all under `https://github.com/Bramvanosta/Ranch/`). `update-feed` is marked pre-release, so it is never shown as the latest release; the first publication creates it.

- **Two channels.** `tauri.conf.json` ships the stable endpoint; `scripts/set-version.mjs` switches a pre-release build to `rc.json`. So an RC is offered the next RC (and the final release), and a stable build is never offered an RC. It also means the whole flow can be rehearsed with two RC tags before a stable release.
- **Who publishes.** The `publish` job, once the release is public, one platform after the other: `scripts/publish-update.mjs <tag> --platforms … --artifact … --signature …` writes `platform-<platform>.json` on the tag's release (pointing at the archive the build job attached), then **rebuilds** the manifest from the platform files present for that tag and writes it to the channels. Run after run, the manifest gains each platform, and a re-run heals one that is missing a platform. A channel is never moved back to an older version. Assets are replaced through a temporary name, so a manifest is never missing while it is rewritten. Without `TAURI_SIGNING_PRIVATE_KEY` there is no signature: the step says so and does nothing.
- **What can update itself.** macOS (the universal app) and the Linux **AppImage**. A `.deb` / `.rpm` install says "Updates off: update with your package manager"; the server image is updated by pulling a tag.
- **In the app.** A check at start, at most once a day (`RANCH_DISABLE_UPDATE_CHECK=1` or `--no-update-check` turns it off). When the feed has something newer the sidebar shows *Update available · vX* (and Settings › Updates says so too); the click opens the release notes and *Install and restart*; nothing installs without it. **The notes are decided at release time** by `publish-update.mjs`, from `CHANGELOG.md` as it is at the tag: the **summary paragraph** of the version's section (what stands between `## [vX.Y.Z]` and its first `###`), as plain text: the dialog does not render Markdown. Without a summary it falls back to the bold lead of each bullet (eight at most); a pre-release has no section, so it just says `Ranch X.Y.Z-rc.N`. `--notes <file>` overrides all of it.
- **Trust.** The archive's signature is verified against the public key compiled into the app (`plugins.updater.pubkey` in `tauri.conf.json`), so whoever controls the downloads can withhold an update, never forge one. The key pair was generated with `pnpm tauri signer generate`; the private key and its password live in the two `TAURI_SIGNING_*` secrets and in a password manager: **lose them and installed apps can never be updated again** (they only trust that key).
- **Public downloads only.** An installed app downloads without credentials: the feed works once the repository is public. (`publish-update.mjs` itself goes through the API, so publishing works on a private repository.)
- Replaying a publication by hand: `GITHUB_TOKEN=$(gh auth token) GITHUB_REPOSITORY=Bramvanosta/Ranch node scripts/publish-update.mjs v0.4.0 --platforms linux-x86_64 --artifact Ranch_0.4.0_amd64.AppImage --signature Ranch_0.4.0_amd64.AppImage.sig` (the release `v0.4.0` must be public).
