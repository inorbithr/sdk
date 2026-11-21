//! Any string as a token: reading its claims never panics.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &str| {
    let _ = inorbithr::login::Claims::read(data, time::OffsetDateTime::UNIX_EPOCH);
});
