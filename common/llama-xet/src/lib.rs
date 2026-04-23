// TEMPORARY probe — deleted in Task 4 once we have real FFI.
// Proves hf-xet compiles and the XetSession symbol is reachable.
//
// Naming gotcha: xet-core's download crate has THREE names.
//   - On-disk directory:  xet_pkg/
//   - Cargo package name: hf-xet   (what the `[dependencies]` table resolves)
//   - Rust library name:  xet      (what `use` refers to — set via [lib] name)
// The xet_pkg/Cargo.toml uses `[lib] name = "xet"`, so Rust code imports
// as `use xet::xet_session::XetSession`.
#[allow(dead_code)]
fn _probe_xet_session_exists() -> &'static str {
    std::any::type_name::<xet::xet_session::XetSession>()
}
