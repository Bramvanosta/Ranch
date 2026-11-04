# CI and releases

Everything runs on GitHub Actions, from the workflows in `.github/workflows/`:

| Workflow | When | What |
|---|---|---|
| `ci.yml` | push to `main`, pull requests | frontend (Biome, types, unit tests, no credentials in SQL migrations); on pull requests: no shipped migration changed; overflow check; Rust with the `server` feature; Rust with the `desktop` feature; on pull requests: the server image, built and smoke-tested |
| `release.yml` | version tags | checks the tag is a commit of `main` whose CI passed, creates the GitHub release as a draft, builds the Linux bundles and the macOS app **without any secret**, signs them on fresh runners of the `release` environment, attaches them, then publishes the release, the updates and the Homebrew casks |
| `image.yml` | push to `main`, version tags | builds the server image for amd64 and arm64, **smoke-tests each**, pushes them to `ghcr.io/bramvanosta/ranch` (a tag: only from a commit of `main` whose CI passed) |

The release and the image use the workflow's own `GITHUB_TOKEN`: there is no token to create or rotate.

## CI

- **frontend**: `bash scripts/check-no-creds-in-sql.sh`, `pnpm lint`, `pnpm typecheck`, `pnpm test` (Vitest and the `node --test` suites of `scripts/`). Node 24, pnpm through corepack at the version of `packageManager`.
- **migrations** (pull requests only): `scripts/check-migrations-unchanged.sh origin/<base>`. sqlx stores the checksum of every migration it applied and refuses to start when a file no longer matches: a migration that shipped never changes, not even a comment. New migrations are always fine; the newest one of the base branch may still change while no `v*` tag contains it.
- **overflow**: `pnpm check:overflow` in the `mcr.microsoft.com/playwright:v<version>-noble` image, which carries Chromium and its libraries. Its version is read from `@playwright/test` in `package.json`, which must stay pinned to an exact version: bumping the package bumps the image.
- **server**: `cargo fmt --check`, clippy and the library tests of every crate of the workspace (`--workspace`) with `--no-default-features --features server`: the binary of the Docker image, with no WebKit and nothing to install. A test here checks that every command is served over HTTP or refused as desktop-only, as the catalogue of operations says.
- **desktop**: clippy on every target of every crate and all the tests (`--workspace`), integration tests included, on Ubuntu with the Tauri system dependencies, against a seeded Redis service and real BullMQ 5 and 6 workers.
- **image** (pull requests only): `docker build`, then `scripts/smoke-test-image.sh`. On `main`, `image.yml` does the same and pushes.

**One Rust everywhere**: `rust-toolchain.toml` names the version (rustup reads it locally), and `mise.toml`, `ci.yml`'s `toolchain:` inputs, `release.yml`'s `rustup toolchain install` steps and the Dockerfile's `rust:` image name the same one. Bump them together, in one commit: clippy gains lints with every release, so a bump can need fixes. The release builds use that version too and build exactly `Cargo.lock` (`--locked`), like CI. The Rust jobs build **without debug info** (`CARGO_PROFILE_DEV_DEBUG=0`): the desktop job links some thirty test binaries that each carry all of Tauri, and with debug info the linker can run out of memory. For the same reason those test binaries are linked three at a time. `Swatinem/rust-cache` keeps the dependencies between runs.

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

`release.yml` runs in seven jobs. The signing keys never share a runner with code from a dependency (see [What the workflow trusts](#what-the-workflow-trusts)), so each platform is **built** by a job that holds no secret, then **signed** by a job on a fresh runner that runs this repository's scripts only:

| Job | Runner | Holds | Does |
|---|---|---|---|
| **release** | `ubuntu-24.04` | the `release` environment, but sees each secret only as *set or not* | checks the commit (above); checks what the environment holds, each set all or nothing (`scripts/release-preflight.mjs`: a partial set fails the run here, naming what is missing, before anything is built); creates the release as a **draft** (a pre-release when the tag has a `-`), or updates it on a re-run. Its notes are the summary paragraph of the version's section of `CHANGELOG.md`, when there is one, followed by how to install. |
| **build-linux** | `ubuntu-22.04` (an older glibc, so the AppImage runs on more distributions) | nothing: no secret, no variable, no environment, `contents: read` | `pnpm install`, cargo, `tauri build`: `.deb`, `.rpm` and `.AppImage`, **without** updater signature (`createUpdaterArtifacts: false`). Packs them in a tar, gives its sha256 as the job's output, uploads it as an artifact. |
| **build-macos** | `macos-14` | nothing, as above | one universal app (Apple Silicon and Intel) in three steps: `tauri build --target universal-apple-darwin --no-bundle`, `lipo` to merge `ranch-mcp` (the universal build only merges the main binary), then `tauri bundle --features desktop --bundles app`. No Developer ID signature, no notarization, no updater archive, no DMG. Packed with `/usr/bin/tar` (the links kept), its sha256 as the output. |
| **sign-linux** | a fresh `ubuntu-24.04` | the `release` environment: the updater key | checks the artifact against build-linux's sha256 before unpacking it, then signs the AppImage (`scripts/minisign.mjs`). |
| **sign-macos** | a fresh `macos-14` | the `release` environment: the Apple set and the updater key | checks the artifact against build-macos's sha256; imports the certificate into a temporary keychain (`scripts/ci-keychain.sh`); `scripts/sign-macos.mjs`: `codesign` inside-out (`ranch-mcp`, then the app; hardened runtime, secure timestamp, no entitlements, as Tauri signed it), notarization of a `ditto` zip with the App Store Connect key, stapling, the updater archive `Ranch.app.tar.gz` made by `/usr/bin/tar` from the stapled app, the DMG made by `hdiutil` (not Tauri's `bundle_dmg.sh`), signed, notarized and stapled; then `codesign --verify --deep --strict`, Gatekeeper (`spctl`: accepted, `source=Notarized Developer ID`) for the app and the DMG, both tickets, `APPLE_TEAM_ID` on every signature. Then the archive's signature (`scripts/minisign.mjs`). The keychain and every key file are removed in a last step that always runs. |
| **attach** | `ubuntu-24.04` | `contents: write`, no signing secret | checks each signing job's files against its sums, then `scripts/attach-release.sh`: `SHA256SUMS-<platform>.txt` and `gh release upload --clobber`. |
| **publish** | `ubuntu-24.04` | the `release` environment: the tap's deploy key | makes the release public (and the latest one, for a stable version), publishes the updates (below), then updates the Homebrew tap. The tap comes last and its failure fails nothing: the release and the updates are out by then. |

The signing jobs run no `pnpm install`, no package script, no cargo, no Homebrew: checkout, Node (`actions/setup-node`), the artifact, then `node scripts/minisign.mjs`, `node scripts/sign-macos.mjs`, `bash scripts/ci-keychain.sh` and the system's tools (`codesign`, `xcrun`, `hdiutil`, `spctl`, `ditto`, `security`, `/usr/bin/openssl`, `/usr/bin/tar`). Those scripts import Node.js built-ins only; `scripts/release-workflow.test.mjs` walks their imports and fails on any package.

**Re-runs.** Re-running a job replaces what it had attached, public release or not: keep the repository's *immutable releases* setting off (the update feed replaces its assets at every release too). One exception: once `publish` has written a platform's `platform-<platform>.json`, installed copies are told to download that platform's archive (`Ranch.app.tar.gz`, the AppImage) and check it against that `.sig`. From then on, a re-run of the signing and attach jobs **keeps the published archive and `.sig`** (`attach-release.sh` downloads them, checks them against the app's public key, lists them in the sums and does not upload them again) and replaces only the other files (the DMG, the `.deb`, the `.rpm`). A re-run can never make the feed serve another binary than the one it already points at; a fixed update is a new version.

Write the CHANGELOG section, with its summary paragraph, **before** tagging the release: the release page and the update dialog both take their text from the tag.

## Secrets: the `release` environment

The signing secrets live in the **`release` environment** (Settings → Environments → `release`), not in the repository's secrets: only the jobs that name the environment (`release`, `sign-linux`, `sign-macos`, `publish`) can read them, and only on a `v*` tag (the environment's deployment rule, below). Each set is optional, and all or nothing: a set with some of its secrets fails the `release` job, naming the missing ones.

| Set | Secrets | Without it |
|---|---|---|
| Apple | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_TEAM_ID`, `APPLE_API_KEY`, `APPLE_API_ISSUER`, `APPLE_API_KEY_P8` | the macOS app is ad-hoc signed and **Gatekeeper refuses it** once downloaded (see below) |
| Updater | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | no updater archive is signed and **no update is published** |
| Tap | `HOMEBREW_TAP_DEPLOY_KEY` | the Homebrew cask is not updated (see below) |

`APPLE_ID` and `APPLE_PASSWORD` are no longer read: notarization uses the App Store Connect API key, whose `.p8` goes to `notarytool` as a file (an app-specific password could only be given on its command line). The `release` job warns while they still exist; delete them.

### The Homebrew tap

[`Bramvanosta/homebrew-tap`](https://github.com/Bramvanosta/homebrew-tap) holds two casks of the universal DMG: `ranch` follows stable releases, `ranch@beta` follows every release, candidates and stable ones, so it is never behind `ranch`. The `publish` job writes the casks of the tag (`scripts/homebrew-cask.mjs`, with the DMG's checksum from `SHA256SUMS-macos.txt`; a cask never moves back to an older version) and pushes it to the tap with `HOMEBREW_TAP_DEPLOY_KEY`, the private half of an SSH deploy key that may write to the tap and to nothing else. A new key: `ssh-keygen -t ed25519 -N "" -f tap_key`, then `gh repo deploy-key add tap_key.pub --repo Bramvanosta/homebrew-tap --allow-write` and `gh secret set HOMEBREW_TAP_DEPLOY_KEY --env release < tap_key`, and delete both files.

The casks download from this repository's releases, so `brew install` works once those are public.

### Signing and notarizing the macOS app

It needs a paid Apple Developer account:

1. developer.apple.com → Certificates → **Developer ID Application**; install it, then export it from Keychain Access **with its private key and Apple's Developer ID intermediate certificate** as a `.p12` with a password.
2. App Store Connect → Users and Access → Integrations → **App Store Connect API** → a *Team key* with the *Developer* role; download its `AuthKey_<id>.p8` (once only) and note its key id and the issuer id.
3. Secrets of the `release` environment: `APPLE_CERTIFICATE` = `base64 -i cert.p12`, `APPLE_CERTIFICATE_PASSWORD` = that password, `APPLE_SIGNING_IDENTITY` = `Developer ID Application: <Name> (<TEAMID>)`, `APPLE_TEAM_ID`, `APPLE_API_KEY` = the key id, `APPLE_API_ISSUER` = the issuer id, `APPLE_API_KEY_P8` = the content of the `.p8`. Delete the files.
4. Tag again: `sign-macos` signs, notarizes and staples; each notarization adds a few minutes.

In the signing job, the certificate goes into a temporary keychain (`scripts/ci-keychain.sh`): the private key is imported **non-extractable** (`-x`, codesign may use it, nothing can export it), every password reaches `security` on its standard input rather than its command line, and the keychain is deleted in a final step that always runs. The `.p8` is a `0600` file under `$RUNNER_TEMP`, removed with it.

An unsigned build is quarantined once downloaded, and macOS answers *"Apple could not verify that Ranch.app is free of malware"*. For a build you trust: `xattr -dr com.apple.quarantine /Applications/Ranch.app`, or System Settings → Privacy & Security → *Open Anyway* right after the refused launch.

## GitHub settings, by hand

The workflow writes none of these; the repository's owner sets them once, in the repository's settings.

1. **The environment.** Settings → Environments → *New environment* → `release`.
   - *Deployment branches and tags* → *Selected branches and tags* → add a rule of type **Tag** with the pattern `v*`, and nothing else: a run from a branch, a pull request or any other tag cannot reach its secrets.
   - *Environment secrets*: the sets above. Then delete the same names from Settings → Secrets and variables → Actions → *Repository secrets*: a repository secret is readable by any job of any workflow that names it, on any branch. Delete `APPLE_ID` and `APPLE_PASSWORD` too.
   - No *required reviewers*, unless you want to approve every release by hand: each of the four jobs that name the environment would wait for it.
2. **A ruleset on the tags.** Settings → Rules → Rulesets → *New tag ruleset*: target the pattern `v*`, enforcement *Active*, rules *Restrict creations*, *Restrict updates* and *Restrict deletions*, and add the *Repository admin* role to the bypass list. Only an admin can then push, move or delete a version tag, which is what deploys to the environment.
3. **Main.** Settings → Rules → Rulesets → *New branch ruleset* on the default branch: *Require a pull request before merging*, *Require status checks to pass* (the jobs of `ci.yml`), *Block force pushes*, *Restrict deletions*. A tag must be a commit of `main` whose CI passed (`scripts/check-release-commit.sh`): main's protection is what makes that mean something.
4. **Immutable releases** stays off (Settings → General → Releases): see *Re-runs* above.

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
- **Who publishes.** The `publish` job, once the release is public, one platform after the other: `scripts/publish-update.mjs <tag> --platforms … --artifact … --signature …` writes `platform-<platform>.json` on the tag's release (pointing at the archive the attach job uploaded), then **rebuilds** the manifest from the platform files present for that tag and writes it to the channels. Run after run, the manifest gains each platform, and a re-run heals one that is missing a platform. A channel is never moved back to an older version. Assets are replaced through a temporary name, so a manifest is never missing while it is rewritten. Without the updater set the `release` job says so and the step does not run; with it, a missing `.sig` fails the step.
- **What can update itself.** macOS (the universal app) and the Linux **AppImage**. A `.deb` / `.rpm` install says "Updates off: update with your package manager"; the server image is updated by pulling a tag.
- **In the app.** A check at start, at most once a day (`RANCH_DISABLE_UPDATE_CHECK=1` or `--no-update-check` turns it off). When the feed has something newer the sidebar shows *Update available · vX* (and Settings › Updates says so too); the click opens the release notes and *Install and restart*; nothing installs without it. **The notes are decided at release time** by `publish-update.mjs`, from `CHANGELOG.md` as it is at the tag: the **summary paragraph** of the version's section (what stands between `## [vX.Y.Z]` and its first `###`), as plain text: the dialog does not render Markdown. Without a summary it falls back to the bold lead of each bullet (eight at most); a pre-release has no section, so it just says `Ranch X.Y.Z-rc.N`. `--notes <file>` overrides all of it.
- **Trust.** The archive's signature is verified against the public key compiled into the app (`plugins.updater.pubkey` in `tauri.conf.json`), so whoever controls the downloads can withhold an update, never forge one. The key pair was generated with `pnpm tauri signer generate`; the private key and its password live in the two `TAURI_SIGNING_*` secrets of the `release` environment and in a password manager: **lose them and installed apps can never be updated again** (they only trust that key).
- **Who signs.** `scripts/minisign.mjs`, in the signing jobs: the minisign format the Tauri CLI writes (BLAKE2b-prehashed Ed25519, the bundler's trusted comment `timestamp:…\tfile:…\tversion:…`), in Node.js alone, so the signing job runs no Tauri CLI. The key is written to a `0600` file for that one command and removed after it. Before writing a `.sig` it checks the key is the one `tauri.conf.json` trusts (a wrong password or another key is refused) and verifies the signature it made.
- **Public downloads only.** An installed app downloads without credentials: the feed works once the repository is public. (`publish-update.mjs` itself goes through the API, so publishing works on a private repository.)
- Replaying a publication by hand: `GITHUB_TOKEN=$(gh auth token) GITHUB_REPOSITORY=Bramvanosta/Ranch node scripts/publish-update.mjs v0.4.0 --platforms linux-x86_64 --artifact Ranch_0.4.0_amd64.AppImage --signature Ranch_0.4.0_amd64.AppImage.sig` (the release `v0.4.0` must be public).

## What the workflow trusts

- It runs on pushed `v*` tags only, never on a pull request, with read-only permissions by default; `contents: write` only on the three jobs that call `gh` (`release`, `attach`, `publish`), which run no build tool; the job token only in the steps that call `gh`. Every action is GitHub's own, pinned to the commit of an exact release (the tag, `# v5.1.0`, written beside it); checkouts keep no credential; no `${{ }}` is expanded inside a script (values come through `env:`).
- **The signing keys never share a runner with code from a dependency.** On a hosted runner every step of a job runs as the same user, and a process one step leaves behind lives until the job ends: it can read the other processes' environments (`ps -E`), the files written for signing, and use an unlocked keychain. So the build jobs, which run the dependencies' code (pnpm's packages and their install scripts, cargo's build scripts and proc macros, the Tauri CLI), hold no secret, no variable and no environment; the signing jobs, which hold the keys, run on fresh runners and run only this repository's scripts, whose imports are Node.js built-ins, and the system's tools.
- **What the build jobs built is what gets signed.** A signing job checks that the artifact is the very file its build job packed (its sha256, passed as a job output), and signs it. It trusts the build itself: a compromised dependency could still put its own code into the app, which would then be signed and notarized. Where the build's inputs come from (the lockfile's packages, `Cargo.lock`, the pnpm store cache, the runner image) is a separate matter, build provenance, that this split does not settle.
- A dependency compromised at build time can no longer steal the Developer ID key or the updater key, nor sign with them anything but the build it is part of: it never runs where they are.
- `scripts/release-workflow.test.mjs` reads the workflow as data and holds it to these rules, each with a mutation of the workflow it must catch; it also runs the step scripts against stand-ins. The signing scripts have their own tests against stand-ins (`sign-macos.test.mjs`, `ci-keychain.test.mjs`, `attach-release.test.mjs`, `minisign.test.mjs`): no test reaches the real keychain, Apple or GitHub.
- **Only a real run proves the whole chain.** Tested with stand-ins only, and so first proven by the first tag after this change: the import of the real `.p12` into a keychain and `set-key-partition-list`; `codesign` with the real identity and timestamp server; notarization (the API key, Apple's answer, stapling); `spctl`'s verdict on the real app and DMG; `hdiutil` on a real runner; the artifact upload and download between jobs; the environment's deployment rule; `gh release` against the real release. A first tag can be a pre-release (`v0.5.0-rc.1`): it goes through every job, and the `rc` channel only.
