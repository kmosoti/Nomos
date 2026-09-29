//! A build script that does nothing. Running it at all is what the
//! control needs: it is user code executed while building.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
}
