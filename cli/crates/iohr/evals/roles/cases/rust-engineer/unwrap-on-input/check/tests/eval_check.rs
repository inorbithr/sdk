//! User input never panics the parser.

#[test]
fn eval_bad_input_does_not_panic() {
    for input in ["", "db.local", "db.local:", ":5432", "db.local:99999", "db.local:http"] {
        let r = std::panic::catch_unwind(|| format!("{:?}", addr::parse(input)));
        assert!(r.is_ok(), "parse({input:?}) panicked");
    }
}

#[test]
fn eval_bad_port_is_an_error() {
    assert!(format!("{:?}", addr::parse("db.local:99999")).starts_with("Err"));
}
