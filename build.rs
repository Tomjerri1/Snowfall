fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        if let Err(e) = winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
        {
            eprintln!("{}", e);
            std::process::exit(1);
        }
        println!("cargo:rerun-if-changed=assets/icon.ico");
    }
}
