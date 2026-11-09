# BiliNook

BiliNook 是 Windows 桌面 App，集中顯示你關注的 Bilibili 創作者動態、投稿影片與直播。介面與背景服務使用 Rust／GPUI，直播影像透過 WebView2 嵌入 B 站官方播放器。

## 使用

從 [Releases](https://github.com/Baiming0722/BiliNook/releases/latest) 下載並解壓 Windows x64 可攜版，開啟 `BiliNook.exe` 即可使用。播放直播需要 [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)。

從「B站特別關注」升級時，請先在舊版托盤選擇退出，再開啟 BiliNook。新版會直接讀取 `%USERPROFILE%\.bilibili-follow\data` 中的追蹤名單、分組與設定，也可匯入舊版 JSON 備份。開發版本預設使用 `.local/data`。

亦可明確指定隔離資料目錄：

```powershell
& '.\dist\BiliNook.exe' --data-dir '.\.local\試用資料'
```

`BILI_DATA_DIR` 也可覆寫預設位置，`--data-dir` 的優先順序最高。`--print-data-dir` 只印出目錄後退出，不讀寫存檔。單例鎖依資料目錄識別；重複開啟相同目錄會喚醒既有視窗。

按 X 隱藏至托盤；左鍵托盤圖示顯示視窗，右鍵提供顯示／隱藏與退出。側欄「退出應用」與 `Ctrl+Q` 也會先排空存檔。`Ctrl+1–4` 切換主頁，`F5` 刷新。設定的「測試系統通知」可檢查 Windows 通知權限與音效。

四個主頁支援動態分類／關鍵字／UP／分組、游標載入、單／雙訊息流、已讀、轉發原文、相簿預覽與評論；影片排序／搜尋、直播名單、UP 備註／標籤／置頂／個別通知、分組成員及 JSON 備份匯出入。窄視窗會以主／副訊息流按鈕切換，保留兩欄狀態。

動態空回應會有限重試；更新失敗時盡量保留已有內容並提示，較舊分頁失敗可重新載入。側欄吉祥物、載入及分組切換使用可停用的 ASCII 動畫。

直播工作台另有房號／網址與批次新增、獨立分組／備註／置頂／排序、七種房間狀態、卡片／清單、進階篩選及預設、快速尋找、觀看清單、七日觀測統計、歷史查詢與 CSV、通知紀錄／勿擾／暫停一小時、移除復原。支援卡片預覽及自適應／四宮格／九宮格／焦點同屏，播放上限可選 1／2／4／9 間；彈幕及全螢幕使用官方播放器。

`Ctrl+K` 開啟直播快速尋找。直播頁未輸入文字時，`R` 重新整理、`S` 搜尋、`A` 新增、`?` 顯示快捷鍵，`Esc` 關閉最上層面板或預覽。最小化及隱藏至托盤保留播放；切至動態、影片、設定或觀測統計即釋放播放器。外觀設定的主題、重點色、字體、字體／卡片縮放與密度套用全程式；動效遵守 Windows 減少動效設定。

首次沒有 `live-watch.json` 時，會唯讀移轉 `BILILIVE_DATA_FILE`，或 `%USERPROFILE%\.bililive-watch\rooms.json`，成功驗證後才寫入新目錄。原檔不會修改，重啟也不重複匯入。失敗可在直播設定重試、手動匯入或略過。首次維持目前全程式外觀，直播設定另可帶入舊外觀。舊瀏覽器的通知紀錄、勿擾及操作音效偏好未存於來源 JSON，因此採新預設。

## 開發

需要 Windows x64、Rust MSVC 工具鏈、Visual Studio Build Tools 的 C++ 工具及 Windows SDK。完整安裝步驟、環境變數與隔離測試方式見 [開發環境](docs/DEVELOPMENT.md)。

```powershell
.\scripts\cargo.ps1 check --workspace
.\scripts\cargo.ps1 test --workspace
.\scripts\cargo.ps1 run -p bilinook
.\scripts\build-release.ps1
```

`scripts/cargo.ps1` 將 Cargo 下載快取放在專案的 `.cargo-home`。一般終端直接使用 `cargo` 亦可。歡迎閱讀 [貢獻指南](CONTRIBUTING.md)；安全問題請參閱 [安全政策](SECURITY.md)。

## 版本

- Rust 2024 edition；本機查驗為 rustc/cargo 1.99.1。
- `gpui-kit = 0.9.0`，提供 `gpui-component = 0.9.0` 與精確相依的 `gpui-pre = 0.3.11`。
- 原始 `gpui = 0.2.2` 與最新元件庫使用不同型別，不混用。程式以 `gpui` 名稱匯入 Kit 的原生 API。
- 官方發布來源：[Kit 0.9.0](https://crates.io/crates/gpui-kit/0.9.0)、[GPUI Kit 原始碼](https://github.com/longbridge/gpui-kit)、[Zed GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)。
- Windows 桌面整合：`tray-icon = 0.26.1`、`windows = 0.62`；播放器為 Windows 限定的 `wry = 0.57.1`／`webview2-com = 0.39`。WebView 在 GPUI 的 STA 主執行緒建立及操作。

## 里程碑

每一階先完成、檢查與測試，再進入下一階。實際結果記錄於 `docs/progress.md`。

0. 環境、Workspace、繁中深色視窗、四分頁。
1. 相容模型、原子存檔、匯入復原、WBI、API、Tokio 與輪詢。
2. 防盜鏈圖片管線、UP 主與分組、直播名單。
3. 完整四視圖互動、虛擬列表、備份與已讀。
4. Windows 托盤、單例、原生通知、退出排空、release。

## 資料隔離

開發與本次全部實測均使用 `.local/` 的獨立副本，未將正式存檔作為測試寫入目標。舊專案 `../bilibili-follow` 僅作唯讀規格來源。

`ups.json`、`settings.json`、`state.json` 保留舊欄位、時間單位及未知欄位；以同目錄暫存檔原子替換，多檔匯入附復原日誌，排隊寫入由核心 actor 持有。另有 `.rust-writer.lock`、`read-dynamics.json` 與 `cache/images/`。舊版已讀保存在瀏覽器端；要帶入這些記錄，請使用舊版匯出的完整備份，於本程式「備份還原」匯入。

單一存檔、復原日誌及 JSON 備份的上限為 32 MiB；超過時會顯示錯誤，並在替換原檔前停止寫入。直播 JSON 備份同樣支援至 32 MiB。

直播以獨立 `live-watch.json` 由同一存檔 actor 管理。整合備份為 v3，包含可選的 `liveWatch`；舊備份不含此欄位時保留直播資料，其中明確指定的 UP 主直播通知開關會同步至對應的直播間。直播設定另支援來源 JSON v1／v2 合併、確認後覆蓋及 v2 匯出。已追蹤 UP 的直播間會自動納入；移除直播監看會記住排除，手動加入的房間則在取消追蹤 UP 後繼續保留。觀測只計算可靠回應覆蓋的區間，查詢失敗不推估開播時長，也不產生假的上下播事件。

## 實作界線

- 公開 B 站 API 可能要求登入或進行風控；會有明確錯誤、備援及估計數量標示，不把失敗偽裝成空資料。網路有逾時、有限重試與取消。
- 群組影片摘要最多讀取 30 位 UP 的近期投稿；超過時顯示提示，可選定個別 UP 載入更多。搜尋／排序作用於已載入影片。動態每欄最多 2000 則、影片最多 3000 部；切換條件會重新查詢。
- 核心圖片記憶體快取限制 32 MiB 編碼資料／128 筆，磁碟 256 MiB／4096 檔／30 天；GPUI 的圖片及 GPU 解碼資源另計。頭像、封面與原圖均由核心下載，附 B 站 Referer／UA，再交給 GPUI。
- Windows Toast 尊重系統通知與音效設定。首次通知會登錄目前使用者的應用顯示名稱；傳送失敗會在視窗顯示原因。通知點擊可在程序常駐期間顯示視窗並開啟 B 站。
- 成品未簽署；已驗證本機 Windows x64。未進行其他電腦、所有網路條件或長時間常駐的驗證。

直播移植的詳細相容契約、真實播放結果、DPI 模擬、效能取樣與實測限制見 [移植與驗證](docs/live-watch-port.md)。內容字級允許 0.5–2 倍，導覽與工具列維持 0.85–1.2 倍，讓窄視窗仍有內容空間。

側欄小電視、各頁標題、未讀動態與底部會顯示 ASCII 動畫；同步、等待和載入仍有狀態提示，批次操作則顯示實際進度。減少動效時保持靜態，也可在「直播工作台 → 設定 → 全程式外觀」關閉「使用 ASCII 動畫」。詳見 [使用及驗證](docs/ascii-ui.md) 與 [技能輔助設計規格](docs/ascii-design.md)。

`bilinook` 套件位於 `crates/bili-gpui`。GPUI 主執行緒透過有界 channel 訂閱 `bili-core`，核心使用獨立的 Tokio Runtime 執行網路請求。詳細實測見 [驗證紀錄](docs/progress.md)。

## 授權

本專案以 [MIT License](LICENSE) 釋出；第三方相依套件及素材適用其各自的授權條款。BiliNook 與 Bilibili 並無官方從屬關係。

