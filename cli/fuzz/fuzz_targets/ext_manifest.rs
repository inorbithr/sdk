//! Any bytes as an extension manifest (the OCI config blob a registry sends): reading
//! it never panics, and what it accepts keeps every field's rule.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if let Ok(m) = iohr::ext::manifest::Manifest::parse(data) {
        assert!(iohr::ext::manifest::check_name(&m.name).is_ok());
        assert!(!m.entrypoint.contains(".."));
        assert!(!m.description.chars().any(char::is_control));
    }
});
