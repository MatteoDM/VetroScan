// SPDX-License-Identifier: GPL-2.0-or-later
//! On Windows, puts the icon and the version in the .exe (what Explorer,
//! the taskbar and "App installate" show).

fn main() {
    let icon = "../../packaging/icons/hp3770-app.ico";
    println!("cargo:rerun-if-changed={icon}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(icon)
        .set("ProductName", "HP Scanjet 3770")
        .set("FileDescription", "HP Scanjet 3770")
        .set("CompanyName", "Matteo Dal Molin")
        .set("LegalCopyright", "GPL-2.0-or-later");
    resource
        .compile()
        .expect("Windows resources (icon, version)");
}
