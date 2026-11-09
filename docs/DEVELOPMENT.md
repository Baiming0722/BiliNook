# 開發環境

## 先決條件

- Windows x64。桌面整合與直播播放器以 Windows 為目標；其他平台尚未驗證。
- [Rustup](https://rustup.rs/) 與 Windows MSVC 工具鏈。根目錄的 `rust-toolchain.toml` 選用 `stable-x86_64-pc-windows-msvc`、rustfmt 與 Clippy；首次使用可能需要下載。`Cargo.lock` 鎖定相依套件版本；stable 編譯器會隨時間更新。
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) 的「使用 C++ 的桌面開發」工作負載，包括 MSVC C++ 編譯工具與 Windows SDK。
- 播放直播需要 [WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)。下載套件及使用 Bilibili 功能需要網路連線。

專案宣告的最低 Rust 版本為 1.92；目前記錄的本機驗證環境為 rustc/cargo 1.99.1，最低版本與其他機器尚未驗證。

## 建置與測試

在專案根目錄開啟 PowerShell：

```powershell
.\scripts\cargo.ps1 check --workspace --locked
.\scripts\cargo.ps1 test --workspace --locked
.\scripts\cargo.ps1 run -p bilinook -- --data-dir .\.local\dev-data
.\scripts\build-release.ps1
```

發行建置輸出為 `dist/BiliNook.exe` 與 `dist/build-info.json`。腳本使用專案內 `.cargo-home` 作為 Cargo 下載快取；也可以直接用 `cargo`，此時 Cargo 採用使用者的預設快取。首次建置需下載 `Cargo.lock` 所列套件。這些快取及 `target/`、`dist/` 都不應提交至 Git。

## 資料與環境變數

| 設定 | 用途 |
| --- | --- |
| `--data-dir PATH` | 指定資料目錄，優先於 `BILI_DATA_DIR`。 |
| `BILI_DATA_DIR` | 覆寫預設資料目錄；不可設為空字串。 |
| `BILILIVE_DATA_FILE` | 指定首次直播資料移轉來源；預設為 `%USERPROFILE%\.bililive-watch\rooms.json`。 |
| `BILI_REDUCE_MOTION=1` | 在 debug 建置中模擬減少動效。 |
| `BILI_ASCII_TRACE` | 在 debug 建置中輸出 ASCII 動畫追蹤。 |
| `BILI_PLAYER_TRACE` | 輸出播放器位置追蹤。 |

這些環境變數由執行程序讀取，專案不會自動載入 `.env`。debug 預設資料目錄是專案內 `.local/data`，release 預設是 `%USERPROFILE%\.bilibili-follow\data`；測試請明確指定新的隔離目錄。首次建立直播資料時仍可能唯讀匯入舊直播資料；若需要完全隔離，也請把 `BILILIVE_DATA_FILE` 指向測試專用來源。使用正式資料前請先退出共用資料的舊程式。

```powershell
$env:BILILIVE_DATA_FILE = Join-Path (Get-Location) '.local\no-legacy-rooms.json'
.\scripts\cargo.ps1 run -p bilinook -- --data-dir .\.local\dev-data
```

不要上傳個人備份、資料目錄、日誌或任何 Cookie、Token。更多功能與資料格式界線見 [README](../README.md)。
