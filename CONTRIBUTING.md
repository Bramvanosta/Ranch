# Contributing to Ranch

Thanks for taking the time. Bug reports, ideas and pull requests are all welcome.

## Before you start

- **A bug?** Open an issue with what you did, what you expected and what happened, your OS and Ranch version (Ranch › About), and your BullMQ and Redis versions if it is about a queue.
- **A feature or a larger change?** Open an issue first, so we can agree on the shape before you spend time on code.
- **A security problem?** Do not open an issue: see [SECURITY.md](SECURITY.md).

## Set up

Follow [Development](README.md#development) in the README: `mise install`, `corepack enable`, `pnpm install`, then `pnpm tauri:dev` for the desktop app or `pnpm dev` for Ranch web, the UI on an in-memory demo world with no Redis. `pnpm lefthook install` registers the pre-commit hooks, which run Biome, `cargo fmt --check` and Clippy on what you commit.

Then read:

- [docs/architecture.md](docs/architecture.md): where things live, and [Adding an operation](docs/architecture.md#adding-an-operation);
- [docs/testing.md](docs/testing.md): the test suites and how to run them, Redis included;
- [docs/glossary.md](docs/glossary.md): the words the code uses;
- [docs/adr/](docs/adr/README.md): the decisions behind the code's shape;
- [docs/design.md](docs/design.md): the design system every screen follows.

## Making a change

- **Keep it focused.** One pull request, one change. Refactors go in their own pull request.
- **Tests come with the change**, at the lowest layer that can hold it: a domain rule beside its code, an operation on the in-memory backend, anything that writes to Redis in the live suites against real BullMQ workers, a component beside it with Vitest. [docs/testing.md](docs/testing.md) says where each kind goes.
- **The UI and MCP stay in step.** Every command is an operation of the catalogue (`src-tauri/crates/application/src/catalogue.rs`), declared read or write with what each shell does with it; a test of each shell fails when the shell and the catalogue differ. Agents never get an operation the UI does not have.
- **Every screen fits.** No clipped, ellipsised or overlapping text, AA contrast in both themes: `pnpm check:overflow` checks it.
- **Generated files are regenerated, not edited**: `src/lib/tauri/bindings.ts` (`cargo test --test codegen`), `src/routeTree.gen.ts` (the Vite plugin), the MCP tool snapshot (`RANCH_UPDATE_SNAPSHOTS=1`). Never edit a migration that has shipped: add one.
- **The CHANGELOG says what changed** for someone using Ranch: add a line under *Unreleased* in [CHANGELOG.md](CHANGELOG.md). Internal changes nobody can see need no entry.

Before you push:

```bash
pnpm check                      # format, lint, types, unit tests
(cd src-tauri && cargo test --workspace)
pnpm check:overflow             # when you touched the UI
```

## What CI runs

Every pull request runs [`.github/workflows/ci.yml`](.github/workflows/ci.yml):

- the front end: `pnpm lint`, `pnpm typecheck`, `pnpm test`, and no credentials in SQL migrations;
- the overflow check, in Playwright's image;
- the Rust workspace with the `server` feature: `cargo fmt --check`, `cargo clippy --workspace --no-default-features --features server --lib --bins -- -D warnings`, `cargo test --workspace --no-default-features --features server --lib`;
- the Rust workspace with the `desktop` feature: `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, then the live suites against Redis and BullMQ 5 and 6 workers;
- the server image, built and smoke-tested.

## Style

- TypeScript and Rust are formatted by Biome and `cargo fmt`; Clippy runs with `-D warnings`.
- Comments say why, not what. Write them, and the UI copy, in plain English.
- A module's doc comment says what it owns and the rules it keeps, in a few lines.
- Comments describe the code as it is: no dates, no "used to", no "for now", no plans. A TODO links an issue.
- A rule that only review enforces belongs in a test or a type.
- UI text is short and concrete: say what happens and how many ("Retry 42 jobs"), never "…" in a button.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
