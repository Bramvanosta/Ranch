fn main() {
    // The HTTP shell (`--no-default-features --features server`) is not a
    // Tauri app: no capabilities, no icons, no `tauri.conf.json` to check.
    #[cfg(feature = "desktop")]
    tauri_build::build()
}
