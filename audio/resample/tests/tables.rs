//! The kernel table against the platform's libm. It was made on Apple silicon,
//! so only that platform has to reproduce it. Other platforms use the table
//! and may find their libm one unit off. That difference is the reason the
//! table exists.

#![cfg(target_vendor = "apple")]

#[test]
fn the_kernel_table_matches_libm() {
    assert_eq!(
        oscilla_resample::reference_kernel(),
        oscilla_resample::kernel()
    );
}

/// Prints `src/tables.rs`. Run with
/// `cargo test -p oscilla-resample --release --test tables -- --ignored --nocapture`
/// on Apple silicon and keep the lines between the markers.
#[test]
#[ignore = "generator"]
fn print_tables() {
    println!("-----BEGIN TABLES-----");
    println!(
        "//! The resampler's step kernel, computed once with Apple's libm on Apple silicon so"
    );
    println!("//! that the output does not depend on the platform's math library.");
    println!("//! `tests/tables.rs` checks it.");
    println!();
    println!("/// The resampler's step kernel: 256 phases of 17 taps.");
    println!("pub(crate) static KERNEL: [i64; 4352] = [");
    for row in oscilla_resample::reference_kernel().chunks(8) {
        let cells: Vec<String> = row.iter().map(|v| v.to_string()).collect();
        println!("    {},", cells.join(", "));
    }
    println!("];");
    println!("-----END TABLES-----");
}
