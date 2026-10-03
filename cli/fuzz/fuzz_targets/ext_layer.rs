//! Any bytes as an extension layer: taking the program out never panics and never
//! returns more than the bound.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(p) = iohr::ext::layer::program(data, "bin/prog") {
        assert!(p.len() as u64 <= iohr::ext::layer::MAX_PROGRAM);
    }
});
