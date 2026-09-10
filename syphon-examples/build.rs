//! Build script for syphon-examples
//!
//! syphon-core handles all Syphon *linking* — it reassembles its bundled
//! Syphon.framework in its OUT_DIR and emits the framework search path and
//! link-lib. What it cannot do is give this crate's binaries an `-rpath`:
//! `cargo:rustc-link-arg` applies only to the emitting package's own targets
//! and does not propagate to dependents. So every crate that produces a
//! binary linking Syphon has to re-emit the runtime search path itself.
//!
//! Because syphon-core declares `links = "Syphon"`, it can hand us the
//! directory it linked against via DEP_SYPHON_FRAMEWORK_DIR. Preferring that
//! over a system-wide install matters: a copy in /Library/Frameworks left by
//! an older installer may predate Apple Silicon and be x86_64-only, and dyld
//! resolves @rpath against the first *match* rather than the first *loadable*
//! one — so a stale copy earlier in the path aborts the launch.
//!
//! This is the pattern downstream crates should copy; see the "Linking from
//! your own crate" section of the README.

fn main() {
    #[cfg(target_os = "macos")]
    {
        // The framework syphon-core actually linked against, exported because
        // it declares `links = "Syphon"`. Cargo sets this for *direct*
        // dependents only, which is why the examples crate depends on
        // syphon-core rather than reaching for the framework by path.
        if let Ok(dir) = std::env::var("DEP_SYPHON_FRAMEWORK_DIR") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
        } else {
            // Fallback for an unusual build where the metadata is missing.
            println!(
                "cargo:warning=DEP_SYPHON_FRAMEWORK_DIR unset; \
                 falling back to a system-wide Syphon.framework"
            );
        }

        // A system-wide install, for anyone who prefers one. Deliberately
        // after the bundled framework so a stale x86_64-only copy cannot
        // shadow it.
        println!("cargo:rustc-link-arg=-Wl,-rpath,/Library/Frameworks");

        // Bundle-relative, so a packaged .app can carry its own copy in
        // Contents/Frameworks.
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../Frameworks");

        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-env-changed=DEP_SYPHON_FRAMEWORK_DIR");
    }
}
