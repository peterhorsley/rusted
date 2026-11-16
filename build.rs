fn main() -> std::io::Result<()> {
    let icon = std::path::Path::new("src").join("assets").join("app.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    if std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo sets CARGO_CFG_TARGET_OS") == "windows" {
        let icon = icon
            .to_str()
            .ok_or_else(|| std::io::Error::other("Icon path is not UTF-8"))?;
        winresource::WindowsResource::new()
            .set_icon(icon)
            .compile()?;
    }
    Ok(())
}
