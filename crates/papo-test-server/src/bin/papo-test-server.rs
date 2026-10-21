//! Runs a test server until its standard input closes.

#[cfg_attr(not(target_family = "wasm"), path = "papo-test-server/native.rs")]
#[cfg_attr(target_family = "wasm", path = "papo-test-server/wasm.rs")]
mod platform;

fn main() -> std::io::Result<()> {
    platform::run()
}
