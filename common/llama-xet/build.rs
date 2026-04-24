use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set by cargo");

    // Walk up from OUT_DIR to locate <target>/<profile>/ — the directory
    // that holds libllama_xet.a. Dropping llama-xet.h next to the staticlib
    // lets CMake declare both as a single OUTPUT from the custom command.
    //
    // Cargo documents OUT_DIR as "build/<pkg>-<hash>/out" within the
    // target directory (see https://doc.rust-lang.org/cargo/reference/
    // environment-variables.html#environment-variables-cargo-sets-for-
    // build-scripts). Walking up 3 ancestors gives us:
    //
    //   OUT_DIR         = <target>/<profile>/build/<pkg>-<hash>/out
    //   ancestors[1]    = <target>/<profile>/build/<pkg>-<hash>
    //   ancestors[2]    = <target>/<profile>/build
    //   ancestors[3]    = <target>/<profile>          ← we want this
    //
    // This has been stable across Cargo versions since ~1.0 but is
    // technically an implementation detail. If a future Cargo reshapes
    // target layout, set LLAMA_XET_HEADER_OUT from CMake and read it
    // here as a more explicit alternative.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR must be set"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR should be nested at least 3 levels under <target>/<profile>")
        .to_path_buf();
    let header_path = profile_dir.join("llama-xet.h");

    let config = cbindgen::Config::from_file(PathBuf::from(&crate_dir).join("cbindgen.toml"))
        .expect("cbindgen.toml is readable");

    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("cbindgen failed to generate bindings")
        .write_to_file(&header_path);

    // Rerun when any source file changes. The `src` directory coverage
    // is important as we grow the FFI surface across multiple modules.
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=build.rs");
}
