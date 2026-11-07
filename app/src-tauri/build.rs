fn main() {
    // The app's icon is built into the program, so new icons (`npm run tauri icon <file>`) need
    // a rebuild; without this, Cargo wouldn't notice they changed and would keep the old one.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
