//! The layers' dependency rule, where the compiler does not hold it.
//!
//! The domain (`crates/domain`, `ranch-domain`) and the application
//! (`crates/application`, `ranch-application`) are crates of their own: each
//! can only name what its manifest lists, and no crate above it, so the
//! compiler refuses a forbidden `use`. What is left to check is those
//! manifests: they list no wire format (specta, schemars), no store (sqlx,
//! redis), no shell (tauri).
//!
//! The adapters reach their store but no other, and derive no wire format,
//! since what crosses a boundary is `contract/`'s: the manifests of the
//! adapter crates are read (the secret stores' lists no wire format, the
//! SQLite stores' no wire format, Redis or Tauri, BullMQ on Redis's no wire
//! format, SQLite or Tauri), and what is left of `core/` and `adapters/`,
//! modules of the app crate, is read from the source. A test fails here,
//! listing each offending manifest entry or line, rather than letting a
//! dependency or a derive slip a layer.
//!
//! The rule and its reasons are in `docs/architecture.md` (The dependency
//! rule).

use std::path::{Path, PathBuf};

/// Crates the domain and the application must not name.
const FORBIDDEN_CRATES: &[&str] = &["specta", "schemars", "sqlx", "redis", "tauri"];

/// The wire formats, which only `contract/`, `commands/` and the MCP
/// shell's tool inputs derive.
const WIRE_CRATES: &[&str] = &["specta", "schemars"];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Does `code` name the crate `name` as a path root (`name::…`, `use name`)?
fn names(code: &str, name: &str) -> bool {
    code.match_indices(name).any(|(at, _)| {
        let before = code[..at].chars().next_back();
        let after = &code[at + name.len()..];
        let starts_a_path = before.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
            && !code[..at].ends_with("::");
        let ends_a_segment = after.starts_with("::")
            || after.starts_with(';')
            || after.starts_with(" as ")
            || after.starts_with(',')
            || after.starts_with('}');
        starts_a_path && ends_a_segment
    })
}

/// The dependencies a member's manifest lists, in every dependency table
/// (`[dependencies]`, `[dev-dependencies]`, `[target.….dependencies]`…).
fn manifest_dependencies(member: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates")
        .join(member)
        .join("Cargo.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut in_table = false;
    let mut found = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_table = line.trim_end_matches(']').ends_with("dependencies");
        } else if in_table && !line.is_empty() && !line.starts_with('#') {
            let name = line.split(['=', '.']).next().unwrap().trim();
            found.push(name.to_string());
        }
    }
    found
}

/// The entries of `member`'s manifest that name one of `crates`.
fn forbidden_dependencies(member: &str, expected: &str, crates: &[&str]) -> Vec<String> {
    let deps = manifest_dependencies(member);
    assert!(
        deps.iter().any(|d| d == expected),
        "the scan reads {deps:?}"
    );
    deps.into_iter()
        .filter(|d| crates.iter().any(|c| d.starts_with(c)))
        .collect()
}

/// Every line under `layer` that names one of `crates`.
fn offenders(layer: &str, crates: &[&str]) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(layer);
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for what in crates.iter().filter(|c| names(code, c)) {
                let shown = file.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap();
                found.push(format!(
                    "{}:{}: names `{what}`: {}",
                    shown.display(),
                    i + 1,
                    code
                ));
            }
        }
    }
    found
}

#[test]
fn the_domain_manifest_names_no_wire_store_or_shell() {
    let found = forbidden_dependencies("domain", "serde", FORBIDDEN_CRATES);
    assert!(
        found.is_empty(),
        "crates/domain/Cargo.toml breaks the dependency rule: {found:?}"
    );
}

#[test]
fn the_application_manifest_names_no_wire_store_or_shell() {
    let found = forbidden_dependencies("application", "ranch-domain", FORBIDDEN_CRATES);
    assert!(
        found.is_empty(),
        "crates/application/Cargo.toml breaks the dependency rule: {found:?}"
    );
}

#[test]
fn the_adapters_keep_to_their_store_and_derive_no_wire_format() {
    let mut manifests = forbidden_dependencies("secrets", "ranch-application", WIRE_CRATES);
    manifests.extend(forbidden_dependencies(
        "sqlite",
        "sqlx",
        &["specta", "schemars", "redis", "tauri"],
    ));
    manifests.extend(forbidden_dependencies(
        "bullmq-redis",
        "redis",
        &["specta", "schemars", "sqlx", "tauri"],
    ));
    assert!(
        manifests.is_empty(),
        "an adapter crate's manifest breaks the dependency rule: {manifests:?}"
    );
    let mut found = offenders("core", WIRE_CRATES);
    found.extend(offenders("adapters", WIRE_CRATES));
    assert!(
        found.is_empty(),
        "core/ or adapters/ name a wire format:\n{}",
        found.join("\n")
    );
}

#[test]
fn the_scan_sees_what_it_looks_for() {
    assert!(names("use specta::Type;", "specta"));
    assert!(names("#[derive(Debug, specta::Type)]", "specta"));
    assert!(names("use schemars;", "schemars"));
    assert!(!names(
        "use crate::adapters::bullmq_redis::redis_info::ServerInfo;",
        "redis"
    ));
    assert!(!names("serde_json::from_str(data)", "serde"));
    assert!(!names("let t = tauri_like::x;", "tauri"));
}
