// Tauri build script. Runs `tauri-build` codegen, which reads tauri.conf.json,
// embeds the frontend assets / dev config, and generates the context consumed
// by `tauri::generate_context!()` in src/lib.rs.
fn main() {
    // The official installer build can bake in the project's ClubLog API key via
    // the CLUBLOG_API_KEY env var (read by `option_env!` in src/lib.rs). Cargo does
    // NOT recompile on an env var changing unless told to — without this directive
    // an incremental build could ship a stale/empty key.
    println!("cargo:rerun-if-env-changed=CLUBLOG_API_KEY");
    tauri_build::build();
}
