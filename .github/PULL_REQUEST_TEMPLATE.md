## What and why

<!-- What this changes for someone using Ranch, and why. Link the issue: "Closes #123". -->

## How it was checked

<!-- Tests added or changed, screens looked at (dark and light), Redis / BullMQ versions if it writes to a queue. -->

- [ ] `pnpm check` and `cargo test --workspace` pass
- [ ] `pnpm check:overflow` passes (if the UI changed)
- [ ] A line under *Unreleased* in `CHANGELOG.md`
- [ ] A new operation has its entry in the catalogue (`application::catalogue`): an MCP tool, UI-only or forbidden
