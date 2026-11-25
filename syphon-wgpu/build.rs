// syphon-core's rpath only reaches syphon-core's own binaries. Give this crate's
// tests and examples one too, from the framework dir syphon-core exports
// (`links = "Syphon"` -> DEP_SYPHON_FRAMEWORK_DIR).
fn main() {
    if let Ok(dir) = std::env::var("DEP_SYPHON_FRAMEWORK_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
