#[test]
fn eval_slug_max_cuts_at_a_word() {
    assert_eq!(slugs::slug_max("Rust in Production Today", 18), "rust-in-production");
    assert_eq!(slugs::slug_max("Rust", 2), "");
    assert_eq!(slugs::slug("A b"), "a-b");
}
