//! Embeds the app icon into the Windows executable (nothing to do elsewhere).

fn main() {
    println!("cargo:rerun-if-changed=texcraft.rc");
    println!("cargo:rerun-if-changed=../../assets/icon/texcraft.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // A missing resource compiler only costs the icon, not the build.
        if let Err(e) =
            embed_resource::compile("texcraft.rc", embed_resource::NONE).manifest_optional()
        {
            println!("cargo:warning=app icon not embedded: {e}");
        }
    }
}
