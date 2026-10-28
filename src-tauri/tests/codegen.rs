//! Tauri-specta codegen test.
//!
//! Regenerates `src/lib/tauri/bindings.ts` from the current command registry
//! by invoking the same `tauri-specta` builder used at app launch. Running
//! `cargo test --test codegen` keeps the bindings file fresh without
//! launching the UI, useful in CI, in lefthook pre-commit, or after editing
//! a command's signature.
//!
//! The test always succeeds when codegen succeeds; if the export call returns
//! an error, the test fails so a broken builder is caught immediately.

#[test]
fn regenerates_typescript_bindings() {
    // Path is relative to the crate root (src-tauri/) so it lands at the
    // same location whether run from cargo or lefthook.
    ranch_lib::commands::specta_builder()
        .export(
            specta_typescript::Typescript::default(),
            "../src/lib/tauri/bindings.ts",
        )
        .expect("tauri-specta bindings export");
}
