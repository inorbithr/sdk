use super::*;
use serde_json::json;

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/host-action")
}

fn read(p: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn the_v1_examples_validate_and_every_v1_invalid_file_is_refused() {
    let mut n = 0;
    for e in std::fs::read_dir(dir().join("v1/examples")).unwrap() {
        let p = e.unwrap().path();
        assert_eq!(validate_v1(&read(&p)), Ok(()), "{}", p.display());
        n += 1;
    }
    assert_eq!(n, 4);
    let mut n = 0;
    for e in std::fs::read_dir(dir().join("v1/invalid")).unwrap() {
        let p = e.unwrap().path();
        assert!(validate_v1(&read(&p)).is_err(), "{}", p.display());
        n += 1;
    }
    assert_eq!(n, 22);
}

/// Every v0 example is a v1 one with `provider: "builtin"`, and every v0 invalid file stays
/// invalid with it: the built-in half of v1 is v0, rule for rule.
#[test]
fn a_builtin_v1_document_is_the_v0_document_with_its_provider() {
    for (sub, ok) in [("schema/examples", true), ("schema/invalid", false)] {
        for e in std::fs::read_dir(dir().join(sub)).unwrap() {
            let p = e.unwrap().path();
            let mut d = read(&p);
            if let Value::Object(m) = &mut d {
                m.insert("provider".into(), json!("builtin"));
            }
            assert_eq!(validate_v1(&d).is_ok(), ok, "{}", p.display());
        }
    }
}

#[test]
fn a_v0_document_is_not_v1_and_reads_as_builtin() {
    let v0 = read(&dir().join("schema/examples/cancel-run.json"));
    assert_eq!(validate_v1(&v0), Err("/provider: required".to_owned()));
    assert_eq!(provider_of(&v0), Ok(Provider::Builtin));
}

#[test]
fn an_extension_provider_reads_with_its_pins_and_errors_name_no_value() {
    let d = read(&dir().join("v1/examples/extension.json"));
    let Ok(Provider::Extension {
        extension,
        version,
        manifest_digest,
    }) = provider_of(&d)
    else {
        panic!("not an extension provider");
    };
    assert_eq!((extension.as_str(), version.as_str()), ("example", "1.2.0"));
    assert!(is_digest(&manifest_digest));
    let bad = read(&dir().join("v1/invalid/param-multi-line.json"));
    let why = validate_v1(&bad).unwrap_err();
    assert!(why.starts_with("/params/worker:"), "{why}");
    assert!(!why.contains("a\nb"));
}
