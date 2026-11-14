//! Two sessions, one store (RFC 0231): a program using the SDK and `iohr` on the same
//! machine open the same profile table and the same store, each with its own handle, as
//! two processes would. What one writes the other reads, and neither loses the other's
//! entry.
#![cfg(feature = "login")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::thread;

use inorbithr::Secret;
use inorbithr::login::{Config, EntryKey, FileStore, Kind, Profile, ProfileName, Storage, Store};

fn key(profile: &str, account: &str) -> EntryKey {
    EntryKey::new(profile.parse().unwrap(), account).unwrap()
}

#[test]
fn what_one_session_stores_the_other_reads_and_each_keeps_its_own_entry() {
    let dir = tempfile::tempdir().unwrap();
    let a = FileStore::new(dir.path().join("secrets"));
    let b = FileStore::new(dir.path().join("secrets"));

    a.set(&key("work", "acc_1"), &Secret::new("from-a".into()))
        .unwrap();
    b.set(&key("ci", "acc_2"), &Secret::new("from-b".into()))
        .unwrap();

    assert_eq!(
        b.get(&key("work", "acc_1")).unwrap().unwrap().expose(),
        "from-a"
    );
    assert_eq!(
        a.get(&key("ci", "acc_2")).unwrap().unwrap().expose(),
        "from-b"
    );

    // A refreshed token replaces the old one for both.
    b.set(&key("work", "acc_1"), &Secret::new("refreshed".into()))
        .unwrap();
    assert_eq!(
        a.get(&key("work", "acc_1")).unwrap().unwrap().expose(),
        "refreshed"
    );

    assert!(a.delete(&key("ci", "acc_2")).unwrap());
    assert!(b.get(&key("ci", "acc_2")).unwrap().is_none());
}

#[test]
fn sessions_writing_at_once_never_leave_a_torn_secret() {
    let dir = tempfile::tempdir().unwrap();
    let root = Arc::new(dir.path().join("secrets"));
    let long_a = "a".repeat(4096);
    let long_b = "b".repeat(4096);
    let writers: Vec<_> = [long_a.clone(), long_b.clone()]
        .into_iter()
        .map(|value| {
            let root = Arc::clone(&root);
            thread::spawn(move || {
                let store = FileStore::new(root.as_ref().clone());
                for _ in 0..50 {
                    store
                        .set(&key("work", "acc_1"), &Secret::new(value.clone()))
                        .unwrap();
                }
            })
        })
        .collect();
    let reader = FileStore::new(root.as_ref().clone());
    while writers.iter().any(|w| !w.is_finished()) {
        if let Some(s) = reader.get(&key("work", "acc_1")).unwrap() {
            assert!(
                s.expose() == &long_a || s.expose() == &long_b,
                "a reader saw a mix of two writes"
            );
        }
    }
    for w in writers {
        w.join().unwrap();
    }
}

#[test]
fn two_sessions_share_one_profile_table() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = Config::load(dir.path()).unwrap();
    first.profiles.insert(
        "work".parse::<ProfileName>().unwrap(),
        Profile::person(
            "acc_1",
            Storage::Keyring,
            "https://auth.inorbit.hr",
            "iohr-cli",
        ),
    );
    first.default = Some("work".parse().unwrap());
    first.save(dir.path()).unwrap();

    // A second session starts later and sees the first's profile.
    let mut second = Config::load(dir.path()).unwrap();
    assert_eq!(
        second.profile("work").map(Profile::kind),
        Some(Kind::Person)
    );
    assert_eq!(second.default().map(ProfileName::as_str), Some("work"));
    second.profiles.insert(
        "ci".parse().unwrap(),
        Profile::new(Kind::Token, "acc_ci", Storage::File),
    );
    second.save(dir.path()).unwrap();

    let third = Config::load(dir.path()).unwrap();
    assert!(third.profile("work").is_some() && third.profile("ci").is_some());
}
