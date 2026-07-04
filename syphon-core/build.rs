fn main() {
    #[cfg(target_os = "macos")]
    macos_link();
}

#[cfg(target_os = "macos")]
fn macos_link() {
    use std::path::Path;

    // The crate ships the framework payload flattened under
    // frameworks/Versions/A (`cargo package` follows symlinks, so the
    // canonical .framework layout cannot be published as-is). Reassemble a
    // real Syphon.framework in OUT_DIR and link against that — this works
    // identically from a repo checkout and from the crates.io tarball.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let payload = Path::new(&manifest_dir).join("frameworks/Versions/A");
    let search_dir = Path::new(&out_dir).join("frameworks");
    let fw = search_dir.join("Syphon.framework");

    let _ = std::fs::remove_dir_all(&fw);
    copy_dir(&payload, &fw.join("Versions/A"));
    symlink("A", &fw.join("Versions/Current"));
    for name in ["Syphon", "Headers", "Modules", "Resources"] {
        symlink(format!("Versions/Current/{name}"), &fw.join(name));
    }

    println!("cargo:rustc-link-search=framework={}", search_dir.display());
    // Dev-time rpath so this crate's own tests/examples can load the
    // framework; shipped apps must still bundle it in Contents/Frameworks
    // or rely on a system-wide install.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", search_dir.display());

    // Fall back to standard system locations (e.g. if the user installed
    // Syphon.framework system-wide from the official installer).
    println!("cargo:rustc-link-search=framework=/Library/Frameworks");
    println!("cargo:rustc-link-search=framework=/System/Library/Frameworks");

    println!("cargo:rustc-link-lib=framework=Syphon");
    println!("cargo:rustc-link-lib=framework=IOSurface");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
    println!("cargo:rustc-link-lib=framework=CoreGraphics");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=MetalKit");
    println!("cargo:rustc-link-lib=framework=OpenGL");

    // With `links = "Syphon"`, direct dependents can locate the reassembled
    // framework via DEP_SYPHON_FRAMEWORK_DIR (e.g. to bundle it into an .app
    // or add their own rpath for dev runs).
    println!("cargo:framework_dir={}", search_dir.display());

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=frameworks/Versions/A");
}

#[cfg(target_os = "macos")]
fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src)
        .unwrap_or_else(|e| panic!("framework payload missing at {}: {e}", src.display()))
    {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), &to).unwrap();
        }
    }
}

#[cfg(target_os = "macos")]
fn symlink(target: impl AsRef<std::path::Path>, link: &std::path::Path) {
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, link).unwrap();
}
