#[test]
fn eval_median() {
    assert_eq!(stats::median(&[10.0, 40.0, 20.0, 30.0]), Some(25.0));
    assert_eq!(stats::median(&[5.0, 1.0, 3.0]), Some(3.0));
    assert_eq!(stats::median(&[]), None);
}
