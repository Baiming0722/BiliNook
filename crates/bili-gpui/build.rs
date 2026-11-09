fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=assets/bilinook.rc");
    let file_version = format!(
        "FILE_VERSION={},{},{},0",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH"),
    );
    let display_version = concat!("DISPLAY_VERSION=", env!("CARGO_PKG_VERSION"));
    embed_resource::compile("assets/bilinook.rc", [file_version.as_str(), display_version])
        .manifest_required()
        .expect("無法嵌入 BiliNook 的 Windows 檔案資訊");

    // Windows 預設主執行緒堆疊僅 1 MiB；未最佳化 debug 組態下 GPUI 元件樹訊框較大，提高至 8 MiB 與 Unix 一致。
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
}
