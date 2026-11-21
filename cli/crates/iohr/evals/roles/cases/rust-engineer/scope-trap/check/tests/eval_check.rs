#[test]
fn eval_small_sizes_are_bytes() {
    assert_eq!(units::human(512), "512 B");
    assert_eq!(units::human(2048), "2.0 KiB");
}
