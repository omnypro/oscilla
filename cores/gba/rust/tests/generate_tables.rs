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
    println!("-----END TABLES-----");
}
