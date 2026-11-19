use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/ferritone.gresource.xml");
    println!("cargo:rerun-if-changed=assets/player-icon.svg");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"))
        .join("ferritone.gresource");
    let status = Command::new("glib-compile-resources")
        .args([
            "--sourcedir=assets",
            "assets/ferritone.gresource.xml",
            "--target",
        ])
        .arg(output)
        .status()
        .expect("Could not run glib-compile-resources; install the GLib build tools");
    assert!(
        status.success(),
        "Could not compile the Ferritone icon resource"
    );
}
