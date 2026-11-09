# 實作與驗證紀錄

## BiliNook 命名（2026-11-09）

Cargo 應用套件為 `bilinook`，Windows 成品為 `dist/BiliNook.exe`。100 項 workspace 測試通過。已從發行檔檢查 Windows 產品名稱與檔案描述、CLI 說明及預設資料位置。以隔離資料實際開啟 App，確認標題列與側欄顯示 BiliNook、舊格式的分組正常顯示。對同一資料目錄啟動舊版時，舊版正常退出並沿用已開啟的程序。

## 播放器建立時的閃退修復（2026-11-08）

未開播房間的實際滑鼠點擊重現 WebView2 建立期間的 GPUI 狀態借用重入；播放器建立改為在狀態借用外序列化處理，並支援待建立項目取消與安全退出。Workspace 92 項測試通過；兩個真實未開播房間、快速收合／重開、立即切頁及退出均已隔離實測，另確認一個真實直播及同屏畫面與播放時間推進。此前的按壓波紋修復處理的是另一個 panic。

## 前次更新：分組載入與 ASCII 對齊（2026-11-08）

上游首頁空回應重試一次，仍為空時保留可用的舊動態並提示；單一成員較舊分頁失敗時保留其他內容並要求重載。切換篩選範圍立即顯示載入狀態。修正側欄吉祥物等寬對齊，加入 Pong 載入及 240ms 擦拭轉場。Workspace 91 項測試通過，使用隔離資料實際載入組別公開動態並檢視一般／窄視窗與轉場；詳見 [ASCII 使用及驗證](ascii-ui.md)。

## 前次更新：圖文動態修復與 ASCII 介面動畫（2026-11-08）

修正 `DYNAMIC_TYPE_DRAW` 採用 `MAJOR_TYPE_OPUS` 時遺失正文與圖片，並支援顯示預約標題、時間及人數；預約操作導向 B 站原動態。側欄、頁面標題、底部與未讀卡片新增共用時鐘的 ASCII 動畫，減少動效時為靜態，關閉選項時隱藏。當時完整 workspace 86 項測試通過；合成圖文的原生 UI、預約卡、原圖預覽、窄視窗、減少動效及關閉選項均已檢查。截圖中的動態最初因公開清單空回應無法驗證，後續隔離測試已取得並顯示。詳見 [ASCII 使用及驗證](ascii-ui.md)。

## 前次更新：直播預覽修復與 ASCII 狀態擴充（2026-11-08）

修正直播卡片「預覽」及「新增監看」共用的按壓波紋在滑鼠事件中呼叫 `request_animation_frame`，造成 GPUI panic 的問題。新增圖片／播放器載入的連線軌道，以及有直播時工作台與其他頁面側欄的小型廣播動畫；沿用共用時鐘、減少動效與原有開關。完整 workspace 84 項測試通過；隔離原生測試確認預覽收合／重開、雙預覽、真實 720p 播放時間推進、減少動效、停用開關與失焦停止。Release 版亦驗證真實影像及預覽重開，並更新 `dist/B站特別關注.exe`；詳見 [ASCII 使用及驗證](ascii-ui.md)。

## 前次交付：技能輔助的小型 ASCII 狀態動畫（2026-11-08）

依使用者指定的 ascii-skills，將原有小轉圈擴充為三種用途明確的 ASCII 動畫，分布於 21 處，另加入批次實際進度條。所有動畫仍共用既有 120ms 時鐘，保持單行及固定小尺寸；不新增依賴或資料欄位。完整 workspace 84 項測試通過，涵蓋字寬、共同週期、減少動效、真實筆數及停止／重試後的計算。原生畫面已確認等待三點、處理軌道、停止後 6/10、十行輸入捲動、閒置停止、明亮窄視窗及開關保存。詳見 [ASCII 驗證](ascii-ui.md) 與 [設計規格](ascii-design.md)。

## BiliLive Watch 移植（2026-11-08）

直播狀態頁已擴充為原生直播工作台。新增獨立直播存檔、首次唯讀移轉、來源與整合備份、UP 關聯／排除、分組／觀看清單、觀測／通知、同屏及官方 WebView2 播放器；既有動態、影片、UP 管理與桌面整合保留。

完整 workspace 檢查及 78 項測試通過，另新增跨檔復原案例通過，合計 79 項；後續原生播放器修正對應的 19 項測試亦通過。已實際操作四房、九房、彈幕、全螢幕／抽屜、托盤續播、換頁停止、外觀與快捷尋找，並模擬 Runtime 缺失及 DPI 轉換。全新隔離目錄成功移轉 22 房／2 組，Windows release 啟動、單例喚醒與安全退出通過。後續狀態及限制以 [直播移植驗證](live-watch-port.md) 為準；下列 Phase 0–4 為較早交付紀錄。

## 規格查閱（2026-11-07）

已唯讀查閱舊專案的型別、WBI/Client/Endpoints、JsonStore、Poller、全部 Services、分組/已讀邏輯、App/FeedWorkspace/FeedView/FeedCard/VideosView/LiveView/SettingsView 與 Electron/ELECTRON.md。

保留契約：camelCase（但 `id_str`、直播資訊 snake_case 必須逐欄保留）、毫秒的動態時間/秒的影片時間、通知的全域與個別開關、初次動態輪詢不通知、直播缺少 UID 時保留先前狀態、已知 ID 各 50 筆、已讀最多 5000 筆、最多 50 分組、備份 `readDynamics`、匯入復原日誌。

## Phase 0

- 環境：Rust 1.99.1 x86_64-pc-windows-msvc；MSVC 14.44.35207；SDK 10.0.26100.0。
- 版本：透過 `cargo info` 實際下載並核對 Kit 0.9.0 / Component 0.9.0 / GPUI-pre 0.3.11 原始碼。
- `cargo check --workspace`：通過，0 錯誤。
- `cargo test --workspace`：通過；此骨架尚無業務單元測試。
- `cargo build -p bili-gpui`：通過，原生 exe 已成功啟動。
- Windows UI Automation 已實際 Invoke 四個導覽按鈕；原生無障礙樹可找到四個繁中 Button。
- 視覺核對：以 PrintWindow 擷取主視窗，截圖位於 `.local/verification/phase0-window.png`。

## Phase 1

- 已完成相容模型、未知欄位保留、原子寫入、可取消呼叫端的序列存檔 actor、多檔復原日誌、資料目錄寫入鎖。
- 已完成 WBI、訪客 Cookie、HTTP 逾時/有界重試/四路並行上限/TTL LRU、個人資料/動態/影片/直播/評論與備援解析。
- 核心建立獨立 Tokio Runtime；GPUI 透過 async-channel 等待結果，不在 UI Executor 上執行 reqwest。
- Poller 的直播與動態計時鏈獨立；通知遵守全域/個別開關、首次動態不通知、去重與 50 筆歷史界線。
- 27 個測試通過，包含本機真實 HTTP I/O 的 UA/Referer/TTL/429 重試，以及取消寫入與故障復原。
- 唯讀複製正式三檔至 `.local/legacy-sample` 後，8 位 UP 主與完整 settings/state 通過欄位無損往返；未寫入正式存檔。
- B 站實際服務尚待後續 UI 接入驗證；離線/本機測試不等同真實服務成功。
- Phase 1 最終 `cargo check --workspace` / `cargo test --workspace` 均通過，0 錯誤。

## Phase 2

- 記憶體 LRU 32 MiB / 128 筆、磁碟快取 256 MiB / 4096 檔 / 30 天；同網址 single-flight、四路共用 I/O 上限、UA/Referer、格式/尺寸驗證、失敗佔位與重試。
- 原生視窗已接入隔離資料、UP 主頭像、分組計數與篩選、置頂排序、直播與離線名單、瀏覽器開啟連結。
- `cargo check --workspace` / `cargo test --workspace` 通過，31 個測試全部成功；原生 exe 建置通過。
- 實際 B 站圖床已下載並保存 8 個頭像快取，畫面中已看到真實頭像；背景輪詢取得 1 位直播中並收到新動態事件。全程使用 `.local/legacy-sample`。
- 原生按鈕驗證：管理頁、直播頁、自訂分組與全部篩選；畫面存於 `.local/verification/phase2-ups.png`。

## Phase 3

- 動態與影片採 GPUI 原生可變高度虛擬列表；支援游標分頁、分組/UP/類型/關鍵字、雙訊息流與窄視窗切換、已讀 5000 筆、轉發原文、圖片預覽與評論分頁。
- UP 增刪、備註/標籤/置頂/個別通知、分組成員管理、主題/輪詢/代理、原生檔案對話框備份還原均已接入核心。
- `cargo check --workspace`、`cargo test --workspace`：0 錯誤，38 個測試全部通過；原生建置通過。
- 真實 B 站動態、評論、直播與圖床已在隔離副本中驗證；影片 API 風控時採備援來源並明示警告，未將估計總數偽裝為完整資料。
- Windows UI Automation 實際完成分組新增/改名/儲存、UP 備註/標籤、原生匯出 8 位 UP/2 組/已讀、再匯入還原，以及影片排序、雙流窄視窗切換。
- PrintWindow 已檢視並修正工具列重疊；最終畫面為 `.local/verification/phase3-videos-final.png`、`phase3-responsive-final.png`；圖片預覽為 `phase3-preview.png`。

## Phase 4

- 原生托盤右鍵選單、X 隱藏、側欄/Ctrl+Q/托盤安全退出；同資料目錄以 Windows Mutex + Event 單例喚醒，先取得單例再建立 Runtime/Store。
- Windows UI Automation 已實際操作托盤的顯示/隱藏及退出；重複程序以 exit code 0 結束，既有 HWND 重新顯示，原程序未被替換。
- 原生 Windows Toast 含點擊回應、音效開關及傳送錯誤；Windows 通知歷史實際查得 `bili-test` / `bili-follow`。首次通知設定項目不存在時，直接嘗試 Show；已停用則明示原因。未確認圖示可修改時，不允許 X 隱藏到不可用的托盤。
- `cargo check --workspace`、`cargo test --workspace`：0 錯誤，40 項測試全通過。新增取消 I/O 同時排空已接受存檔的測試；寫入失敗即使等待者取消，也會由 flush 回報，保留舊資料。
- 原生介面已實測儲存淺色/深色並檢視 PrintWindow 輸出，截圖 `.local/verification/phase4-light.png`。跟隨系統會訂閱 Windows 外觀變化。
- Windows 目標實際 Cargo 相依樹未包含 Electron、Node、WebView、wry、tao 或 Tauri runtime；`tray-icon` 僅為原生托盤庫。
- 21 個舊專案關鍵檔案與 3 個正式存檔 SHA-256 均與開工時相同；比對紀錄 `.local/verification/read-only-audit.json`。所有測試只寫 `.local/`。
- Release 首次執行新建置腳本曾遭 Windows 存取拒絕；未更動系統權限，降低 Cargo 並行度後正常繼續。
- `cargo build --release -p bili-gpui` 成功（optimized，8 分 30 秒）；已交付 `dist/B站特別關注.exe`，SHA-256 與編譯輸出一致，另附 `dist/build-info.json`。
- 成品於全新 `.local/release-validation` 實際啟動，X 隱藏、重複程序退出並喚醒原視窗均通過；透過原生 UI 從真實 B 站 API 新增 1 位 UP，完整名稱/頭像落盤成功；原生畫面已檢視，側欄安全退出亦成功。測試未使用正式目錄。

