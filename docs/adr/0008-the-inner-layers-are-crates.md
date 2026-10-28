# 0008. The inner layers are crates

## Context

The layers depend inwards: the domain names no store, shell or wire format and no layer above it, and the application names the domain and nothing outside it. While every layer was a module of one crate, `ranch_lib`, only a test kept them apart, by reading the source for forbidden names. A test of names misses what it does not look for (a re-export, a macro, a path spelled another way), and each new rule meant another pattern to scan for.

## Decision

The domain (`src-tauri/crates/domain`, `ranch-domain`) and the application (`src-tauri/crates/application`, `ranch-application`) are crates of a Cargo workspace. The app crate, `ranch`, stays at `src-tauri/`, where the Tauri CLI expects it, as the workspace root, with one `Cargo.lock` and one `target/`; it keeps the contract, the commands, the shells, the server, the composition roots and the three binaries, and re-exports the two crates as `domain` and `application`.

A crate names only what its manifest lists and never a crate that depends on it, so the rule holds at compile time. What the domain needed from the app crate moved into it: `RanchError`, the audit types with `Caller` and `ApprovalGrant`, the SSH tunnel settings. `RanchError` carries a store's error boxed instead of naming sqlx or redis-rs. The secret store's trait moved into the application's ports. A `From` conversion between a DTO and a domain type lives with the DTO, in the app crate.

The secret stores (`src-tauri/crates/secrets`, `ranch-secrets`), which reach the rest of Ranch through their port alone, are a crate too, and each shell enables the store it needs (`os-keychain` for the desktop app and `ranch-mcp`, `file` for Ranch server). So are the SQLite stores (`src-tauri/crates/sqlite`, `ranch-sqlite`), with the database, its migrations and the audit writer: of the approval queue, they hold the one transition the write pipeline trades an approval with (`approved → consumed`), its SQL unchanged, and the rest of the queue, with the policy around it, is the app crate's. So is BullMQ on Redis (`src-tauri/crates/bullmq-redis`, `ranch-bullmq-redis`), with the connection actors, the BullMQ schema, the SSH tunnel and TLS: it reads a saved connection into its actor's settings through the connection and secret ports, and the composition roots hand it the write pipeline, so it names no database. The adapters left in the app crate are small, and the desktop's progress sink is Tauri's. `tests/dependency_rules.rs` keeps the rest: the domain's and the application's manifests list no forbidden crate, the secret stores' no wire format, the SQLite stores' no wire format, Redis or Tauri, BullMQ on Redis's no wire format, SQLite or Tauri, and what is left of `core/` and `adapters/` derives none.

## Consequences

- A forbidden `use` in the domain, the application or an adapter crate is a compile error, wherever it is spelled.
- Every cargo command covers the workspace with `--workspace` (CI, the pre-commit hook, the documented commands); the Docker build copies `src-tauri/crates/`.
- `ApprovalGrant::new` is public across the crate boundary where it was visible to `core::mcp` alone. The gate is still its one caller, and a grant is still worthless without its approved row, which the write pipeline consumes.
- In an adapter, a store's error is wrapped explicitly (`.map_err(RanchError::database)`, `migration`, `redis`) instead of converted by `?`.
- An adapter becomes a crate of its own once it reaches the rest of the app only through ports.
- The SSH password's keychain name (`ssh_tunnel::ssh_keychain_name`) is public in the BullMQ crate, where it was private to the connection commands: they write the entry and the backend reads it, so both name it from one place. It is a name, and grants nothing.
