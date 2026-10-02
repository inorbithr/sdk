//! Any bytes as a config file: parsing never panics.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = iohr_auth::Config::parse(text, std::path::Path::new("config.toml"));
    }
});
