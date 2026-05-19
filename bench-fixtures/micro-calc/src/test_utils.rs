/// Assert that two f64 values are within 1e-10 of each other.
pub fn assert_close(a: f64, b: f64) {
    let diff = (a - b).abs();
    assert!(diff < 1e-10, "{} != {}", a, b);
}

/// Run basic arithmetic tests using the ops module.
pub fn run_tests() {
    assert_close(super::ops::add(2.0, 3.0), 5.0);
    assert_close(super::ops::subtract(10.0, 3.0), 7.0);
    assert_close(super::ops::multiply(4.0, 5.0), 20.0);
    assert_close(super::ops::divide(15.0, 3.0), 5.0);
}
