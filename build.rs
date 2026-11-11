fn main() {
    println!("cargo:rerun-if-changed=assets/whispertype.rc");
    println!("cargo:rerun-if-changed=assets/whispertype.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/whispertype.rc", embed_resource::NONE)
            .manifest_required()
            .expect("Could not embed the Windows app icon");
    }
}
