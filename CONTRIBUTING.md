# Contributing to Ranch

Thanks for taking the time. Bug reports, ideas and pull requests are all welcome.

## Before you start

- **A bug?** Open an issue with what you did, what you expected and what happened, your OS and Ranch version (Ranch › About), and your BullMQ and Redis versions if it is about a queue.
- **A feature or a larger change?** Open an issue first, so we can agree on the shape before you spend time on code.
- **A security problem?** Do not open an issue: see [SECURITY.md](SECURITY.md).

## Set up

Follow [Development](README.md#development) in the README: `mise install`, `corepack enable`, `pnpm install`, then `pnpm tauri:dev` for the desktop app or `pnpm dev` for Ranch web, the UI with an in-memory demo world and no Redis.

[docs/architecture.md](docs/architecture.md) explains where things live; [docs/design.md](docs/design.md) is the design system every screen follows.

## Making a change

- **Keep it focused.** One pull request, one change. Refactors go in their own pull request.
- **Tests come with the change.** Vitest next to the component or hook, `cargo test` for the Rust core. Anything that writes to Redis is checked against real BullMQ workers in the live suites (see the README).
- **The UI and MCP stay in step.** A new Tauri command goes into `src-tauri/src/commands/registry.rs`, and the `parity_contract` test fails until it has its MCP equivalent or is marked UI-only.
- **Every screen fits.** No clipped, ellipsised or overlapping text, AA contrast in both themes: `pnpm check:overflow` checks it.
- **The CHANGELOG says what changed** for someone using Ranch: add a line under *Unreleased* in [CHANGELOG.md](CHANGELOG.md).

Before you push:

```bash
pnpm check                      # format, lint, types, unit tests
(cd src-tauri && cargo test)
pnpm check:overflow             # when you touched the UI
```

The pre-commit hooks (lefthook) run Biome, `cargo fmt` and Clippy on what you commit.

## Style

- TypeScript and Rust are formatted by Biome and `cargo fmt`; Clippy runs with `-D warnings`.
- Comments say why, not what. Write them, and the UI copy, in plain English.
- UI text is short and concrete: say what happens and how many ("Retry 42 jobs"), never "…" in a button.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
