//! Compile-time configuration shared by papo targets.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(browser)");
    println!("cargo::rustc-check-cfg=cfg(sockets)");
    println!("cargo::rustc-check-cfg=cfg(tls)");

    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();
    let target_family = std::env::var("CARGO_CFG_TARGET_FAMILY").unwrap();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let browser = target_arch == "wasm32" && target_os == "unknown";
    let sockets = target_family != "wasm" || (target_os == "wasi" && target_env != "p1");

    if browser {
        println!("cargo::rustc-cfg=browser");
    }
    if sockets {
        println!("cargo::rustc-cfg=sockets");
    }
    if !browser
        && (std::env::var_os("CARGO_FEATURE_TLS_RING").is_some()
            || std::env::var_os("CARGO_FEATURE_TLS_AWS_LC").is_some())
    {
        println!("cargo::rustc-cfg=tls");
    }
}
