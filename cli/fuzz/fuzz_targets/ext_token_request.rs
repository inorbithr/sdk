//! Any bytes as a request on the extension token channel: the head and body readers
//! never panic, and no scope outside the manifest's is ever accepted.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let (head, body) = data.split_at(data.len() / 2);
    let _ = iohr::ext::socket::parse_head(head);
    let allowed = vec!["agents:write".to_owned(), "domains:read".to_owned()];
    if let Ok(scopes) = iohr::ext::socket::requested_scopes(body, &allowed) {
        assert!(scopes.iter().all(|s| allowed.contains(s)));
    }
});
