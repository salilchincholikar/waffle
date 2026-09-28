//! Generates `include/waffle.h` (the C header the macOS app imports) from this crate.

fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    let config = cbindgen::Config::from_file(format!("{dir}/cbindgen.toml")).expect("cbindgen.toml");
    match cbindgen::Builder::new().with_crate(&dir).with_config(config).generate() {
        Ok(b) => {
            b.write_to_file(format!("{dir}/include/waffle.h"));
        }
        // Don't fail the build on a parse hiccup; the checked-in header is still there.
        Err(e) => println!("cargo:warning=cbindgen: {e}"),
    }
}
