#[cfg(feature = "bundled")]
extern crate augeas_src;
extern crate bindgen;
#[cfg(feature = "pkg-config")]
extern crate pkg_config;

use std::env;
use std::path::{PathBuf};

#[cfg(feature = "bundled")]
fn build_bundled() -> Vec<String> {
    let artifacts = augeas_src::build().expect("autotools build");
    artifacts.print_cargo_metadata();
    vec![artifacts.include_dir().to_str().unwrap().to_string()]
}

#[cfg(feature = "pkg-config")]
fn build_shared() -> Vec<String> {
    let augeas = pkg_config::Config::new()
        .atleast_version("1.13.0")
        .probe("augeas")
        .unwrap();

   augeas
        .include_paths
        .iter()
        .map(|path| format!("-I{}", path.display())).collect()
}

fn main() {
    #[cfg(feature = "bundled")]
    let include_paths = build_bundled();

    #[cfg(feature = "pkg-config")]
    let include_paths = build_shared();

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_args(include_paths)
        .allowlist_function("aug_.*")
        .allowlist_type("aug_.*")
        .allowlist_var("aug_.*")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
