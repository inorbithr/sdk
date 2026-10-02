//! Any bytes as an error answer: reading it never panics and never echoes more than
//! the bounded message.
#![no_main]

libfuzzer_sys::fuzz_target!(|input: (u16, &[u8])| {
    let (status, body) = input;
    let p = iohr::api::Problem::parse(status, body, None);
    assert!(p.message.chars().count() <= 301);
    let _ = p.to_string();
});
