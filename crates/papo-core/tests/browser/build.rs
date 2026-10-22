//! Configures the browser-only test crate.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(browser)");
    println!("cargo::rustc-cfg=browser");
}
