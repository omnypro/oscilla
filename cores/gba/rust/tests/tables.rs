//! The constant tables against the platform's libm. They were made on Apple
//! silicon, so only that platform has to reproduce them. Other platforms use
//! the tables and may find their libm one unit off. That difference is the
//! reason the tables exist.

#![cfg(target_vendor = "apple")]

#[test]
fn the_sincos_table_matches_libm() {
    for i in 0..256u32 {
        let (s, c) = oscilla_gba::reference_sincos14(i << 8);
        assert_eq!((s, c), oscilla_gba::sincos14_table(i << 8), "angle {i}");
    }
}

#[test]
fn the_kernel_table_matches_libm() {
    assert_eq!(oscilla_gba::reference_kernel(), oscilla_gba::kernel_table());
}
