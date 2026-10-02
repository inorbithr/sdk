//! Any bytes as the request a browser (or anything else on this machine) sends to the
//! sign-in listener: reading it never panics.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = iohr_auth::loopback::parse_request(data);
});
