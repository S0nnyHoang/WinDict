// Nhúng manifest: Common Controls v6 (control kiểu mới, chữ gợi ý trong ô nhập) và DPI.
use embed_manifest::manifest::DpiAwareness;
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_manifest(new_manifest("EVDict").dpi_awareness(DpiAwareness::PerMonitorV2))
            .expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
