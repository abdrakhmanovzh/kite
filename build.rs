//! Links libvpx (video stickers) statically, so the app carries no
//! dependency on Homebrew. Looks in LIBVPX_DIR, pkg-config's libdir, then
//! Homebrew's prefixes; `brew install libvpx` provides it. The archive is
//! copied to OUT_DIR as libkitevpx.a: next to Homebrew's libvpx.dylib the
//! linker would otherwise pick the shared library.

use std::{path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=LIBVPX_DIR");
    let pkg_config = Command::new("pkg-config")
        .args(["--variable=libdir", "vpx"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    let found = std::env::var("LIBVPX_DIR")
        .ok()
        .into_iter()
        .chain(pkg_config)
        .chain(["/opt/homebrew/lib".to_owned(), "/usr/local/lib".to_owned()])
        .map(PathBuf::from)
        .find(|dir| dir.join("libvpx.a").exists());
    let Some(dir) = found else {
        println!("cargo:warning=libvpx.a not found; install it with `brew install libvpx`");
        return;
    };
    let source = dir.join("libvpx.a");
    println!("cargo:rerun-if-changed={}", source.display());
    let Some(out) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    if let Err(error) = std::fs::copy(&source, out.join("libkitevpx.a")) {
        println!("cargo:warning=could not copy {}: {error}", source.display());
        return;
    }
    println!("cargo:rustc-link-search=native={}", out.display());
}
