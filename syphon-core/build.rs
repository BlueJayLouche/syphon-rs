fn main() {
    #[cfg(target_os = "macos")]
    {
        // The Syphon.framework binary is bundled inside this crate under frameworks/.
        // Use CARGO_MANIFEST_DIR so it works whether building locally or from crates.io.
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let bundled = std::path::Path::new(&manifest_dir).join("frameworks");

        println!("cargo:rustc-link-search=framework={}", bundled.display());
        // rpath so the linker embeds the load path; apps still need the framework
        // installed in /Library/Frameworks or bundled in their .app at runtime.
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", bundled.display());

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

        println!("cargo:rerun-if-changed=build.rs");
    }
}
