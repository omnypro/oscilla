//! Prints `src/tables.rs`. Run on Apple silicon with
//! `cargo test -p oscilla-gba --release --test generate_tables -- --ignored --nocapture`
//! and keep the lines between the markers.

#[test]
#[ignore = "generator"]
fn print_tables() {
    println!("-----BEGIN TABLES-----");
    println!("//! Constants computed once with Apple's libm on Apple silicon, so that the");
    println!("//! output does not depend on the platform's math library. `tests/tables.rs`");
    println!("//! checks them.");
    println!();
    println!("/// BIOS sine and cosine of an angle's upper byte, scaled by 1 << 14.");
    println!("pub(crate) const SINCOS14: [(i32, i32); 256] = [");
    for i in 0..256u32 {
        let (s, c) = oscilla_gba::reference_sincos14(i << 8);
        println!("    ({s}, {c}),");
    }
    println!("];");
    println!();
    println!("/// The resampler's step kernel: 256 phases of 17 taps.");
    println!("pub(crate) static KERNEL: [i64; 4352] = [");
    for row in oscilla_gba::reference_kernel().chunks(8) {
        let cells: Vec<String> = row.iter().map(|v| v.to_string()).collect();
        println!("    {},", cells.join(", "));
    }
    println!("];");
    println!("-----END TABLES-----");
}
