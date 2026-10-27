fn main() {
    // A release build without `custom-protocol` embeds no frontend at all: the
    // binary starts and then tries to reach a development server that is not
    // running, which looks like a broken app rather than a broken build. Failing
    // here turns that into a message at build time.
    let release = std::env::var("PROFILE").as_deref() == Ok("release");
    let embedded = std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some();
    if release && !embedded {
        panic!(
            "release builds must enable the custom-protocol feature so the \
             frontend is embedded: cargo build --release --features custom-protocol"
        );
    }
    tauri_build::build()
}
