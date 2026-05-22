fn main() {
    println!("cargo:rerun-if-changed=sorotte-gui.rc");
    println!("cargo:rerun-if-changed=sorotte-gui.exe.manifest");
    println!("cargo:rerun-if-changed=assets/icons/sorotte-icon.ico");

    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") {
        embed_resource::compile("sorotte-gui.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("sorotte-gui Windows resources should compile");
    }
}
