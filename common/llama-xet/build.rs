use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR must be set by cargo");

    // Walk up from OUT_DIR to locate <target>/<profile>/ — the directory
    // that holds libllama_xet.a. Dropping llama-xet.h next to the staticlib
    // lets CMake's ExternalProject_Add declare both as BUILD_BYPRODUCTS
    // with a single ${LLAMA_XET_PATH} prefix.
    //
    // OUT_DIR layout: <target>/<profile>/build/<pkg>-<hash>/out
    //                 ^^^^^^^^^^^^^^^^^^^^ we want this
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
