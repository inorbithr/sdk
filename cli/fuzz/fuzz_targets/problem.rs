//! Any bytes as an error answer: reading it never panics and never echoes more than
//! the bounded message.
#![no_main]

use inorbithr::{ApiError, Headers, RawResponse};

libfuzzer_sys::fuzz_target!(|input: (u16, &[u8])| {
    let (status, body) = input;
    let raw = RawResponse::for_tests(status, Headers::default(), body.to_vec());
    let e = ApiError::parse(raw);
    assert!(e.message.chars().count() <= 301);
    let _ = e.to_string();
});
