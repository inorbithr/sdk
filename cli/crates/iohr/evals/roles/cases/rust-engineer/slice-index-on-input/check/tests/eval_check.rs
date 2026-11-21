//! Bytes off the network never panic the frame reader.

#[test]
fn eval_short_input_does_not_panic() {
    for buf in [&[][..], &[0][..], &[0, 9, b'a'][..], &[255, 255][..]] {
        let r = std::panic::catch_unwind(|| format!("{:?}", frames::first_frame(buf)));
        assert!(r.is_ok(), "first_frame({buf:?}) panicked");
    }
}
