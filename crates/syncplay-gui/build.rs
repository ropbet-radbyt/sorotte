fn main() {
    println!("cargo:rerun-if-changed=syncplay-gui.rc");
    println!("cargo:rerun-if-changed=syncplay-gui.exe.manifest");
    println!("cargo:rerun-if-changed=assets/icons/syncplay-icon.ico");

    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") {
        embed_resource::compile("syncplay-gui.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("syncplay-gui Windows resources should compile");
    }
}
