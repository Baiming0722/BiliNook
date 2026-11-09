# 參與 BiliNook

歡迎回報問題、討論改進或提交 Pull Request。開始前請閱讀 [開發環境](docs/DEVELOPMENT.md)，並使用獨立資料目錄測試；不要用正式追蹤資料當作測試資料。

## 回報問題

請在 [Issues](https://github.com/Baiming0722/BiliNook/issues) 說明版本、Windows 版本、重現步驟、預期與實際結果；播放問題請補充 WebView2 Runtime 是否已安裝。截圖及日誌請先移除個人資訊、Cookie、Token、追蹤清單與本機路徑。安全漏洞請依 [安全政策](SECURITY.md) 回報，不要公開可利用的細節。

## 提交變更

1. 先搜尋既有 issue 或 Pull Request；較大的介面、相容性或資料格式變更建議先討論。
2. 建立分支並聚焦於單一問題，保留現有資料格式與可移轉性；需要破壞性變更時說明影響與遷移方式。
3. 依變更執行適當測試。涉及 Rust 邏輯時，至少執行 `./scripts/cargo.ps1 test --workspace --locked`；介面變更請實際檢查相關互動、狀態與無障礙體驗。
4. Pull Request 請說明動機、行為變化、測試結果及已知限制；介面變更可附去識別化截圖。

提交的內容必須有權以本專案的 [MIT License](LICENSE) 提供；第三方素材請保留原有授權與來源聲明。不要提交 `.local/`、編譯產物、帳號憑證或匯出的個人備份。
