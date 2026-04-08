fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=doomgeneric");
    println!("cargo:rerun-if-changed=assets");
}
