// The web UI is embedded from ../web/out. Without this script Cargo would not
// notice that folder appearing or changing, and the server would keep the
// "UI not built" page it was compiled with.
fn main() {
    println!("cargo:rerun-if-changed=../web/out");
    let built = std::path::Path::new("../web/out/index.html").exists();
    println!("cargo:rustc-env=LUMINGO_UI_BUILT={}", u8::from(built));
}
