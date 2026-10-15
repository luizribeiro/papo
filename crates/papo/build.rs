//! Compile-time configuration shared by papo targets.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(tls)");
    if std::env::var_os("CARGO_FEATURE_TLS_RING").is_some()
        || std::env::var_os("CARGO_FEATURE_TLS_AWS_LC").is_some()
    {
        println!("cargo::rustc-cfg=tls");
    }
}
