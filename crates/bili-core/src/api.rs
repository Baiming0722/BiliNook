use crate::{groups::MAX_SAFE_UID, models::*, now_ms, wbi};
use anyhow::{Context, Result, anyhow, bail, ensure};
use futures::future::BoxFuture;
use lru::LruCache;
use reqwest::{
    Client, Method,
    cookie::Jar,
    header::{self, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    num::NonZeroUsize,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use url::Url;

mod parse;
pub(crate) use parse::{number, string, url_string};
pub use parse::{parse_comment, parse_dynamic, parse_profile};

pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
pub const REFERER: &str = "https://www.bilibili.com/";
const API: &str = "https://api.bilibili.com";
const LIVE_API: &str = "https://api.live.bilibili.com";
const SPACE_ORIGIN: &str = "https://space.bilibili.com";
const DM_IMG_STR: &str = "V2ViR0wgMS4wIChPcGVuR0wgRVMgMi4wIENocm9taXVtKQ";
const DM_COVER_IMG_STR: &str = "QU5HTEUgKEludGVsLCBJbnRlbChSKSBVSEQgR3JhcGhpY3MgRGlyZWN0M0QxMSB2c181XzAgcHNfNV8wLCBEM0QxMSlHb29nbGUgSW5jLiAoSW50ZWwp";
const DM_IMG_INTER: &str = r#"{"ds":[],"wh":[3557,2606,57],"of":[162,324,162]}"#;
const DYNAMIC_FEATURES: &str = "itemOpusStyle,listOnlyfans,opusBigCover,onlyfansVote,decorationCard,onlyfansAssetsV2,forwardListHidden,ugcDelete";
const SPACE_WBI_PACE: Duration = Duration::from_millis(320);

pub trait BiliApi: Send + Sync {
    fn room(&self, _id: u64) -> BoxFuture<'_, Result<crate::live_watch::ObservedRoom>> {
        Box::pin(async { anyhow::bail!("此 API 未提供直播房間查詢") })
    }
    fn live(&self, uids: Vec<u64>) -> BoxFuture<'_, Result<BTreeMap<u64, LiveStatusInfo>>>;
    fn dynamics(&self, mid: u64, offset: String) -> BoxFuture<'_, Result<DynamicPage>>;
}

struct WbiKeys {
    img: String,
    sub: String,
    fetched: Instant,
}
type JsonCache = LruCache<String, (Instant, Value)>;

pub struct ApiClient {
    http: Client,
    cookies: Arc<Jar>,
    pace_wbi: bool,
    pub(crate) gate: Arc<Semaphore>,
    cache: Mutex<JsonCache>,
    keys: Mutex<Option<WbiKeys>>,
    visitor: Mutex<Option<Instant>>,
    wbi_pacer: Mutex<Option<Instant>>,
}

impl ApiClient {
    pub fn new(proxy: Option<&str>) -> Result<Self> {
        Self::build(proxy, true)
    }

    fn build(proxy: Option<&str>, https_only: bool) -> Result<Self> {
        let cookies = Arc::new(Jar::default());
        let mut headers = HeaderMap::new();
        headers.insert(header::REFERER, HeaderValue::from_static(REFERER));
        headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-TW,zh;q=0.9,en;q=0.8"),
        );
        let mut builder = Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .cookie_provider(cookies.clone())
            .https_only(https_only)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("重新導向次數過多");
                }
                if allowed_bili_host(attempt.url().host_str().unwrap_or_default()) {
                    attempt.follow()
                } else {
                    attempt.error("已拒絕非 B 站重新導向")
                }
            }));
        if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
            let parsed = Url::parse(proxy).map_err(|_| anyhow!("代理網址格式無效"))?;
            ensure!(
                matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h"),
                "代理協定不支援"
            );
            builder =
                builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| anyhow!("代理設定無效"))?);
        }
        Ok(Self {
            http: builder.build().context("無法建立 B 站 HTTP 客戶端")?,
            cookies,
            pace_wbi: https_only,
            gate: Arc::new(Semaphore::new(4)),
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(1000).unwrap())),
            keys: Mutex::new(None),
            visitor: Mutex::new(None),
            wbi_pacer: Mutex::new(None),
        })
    }

    pub fn http(&self) -> &Client {
        &self.http
    }

    fn set_bili_cookie(&self, name: &str, value: &str) {
        if value.is_empty()
            || value
                .bytes()
                .any(|b| b < 0x21 || b == b';' || b == b'\"' || b == b'\\')
        {
            return;
        }
        if let Ok(origin) = Url::parse(API) {
            self.cookies.add_cookie_str(
                &format!("{name}={value}; Domain=.bilibili.com; Path=/"),
                &origin,
            );
        }
    }

    async fn pace_space_wbi(&self) {
        if !self.pace_wbi {
            return;
        }
        let mut last = self.wbi_pacer.lock().await;
        if let Some(previous) = *last {
            let elapsed = previous.elapsed();
            if elapsed < SPACE_WBI_PACE {
                tokio::time::sleep(SPACE_WBI_PACE - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    async fn bootstrap(&self) -> Result<()> {
        let mut fetched = self.visitor.lock().await;
        if fetched.is_some_and(|time| time.elapsed() < Duration::from_secs(6 * 3600)) {
            return Ok(());
        }
        let response = self
            .http
            .get(REFERER)
            .send()
            .await
            .context("訪客 Cookie 連線失敗")?;
        ensure!(
            response.status().is_success(),
            "訪客 Cookie HTTP {}",
            response.status()
        );
        drop(response);

        let now = now_ms();
        let ts_sec = now / 1000;
        self.set_bili_cookie("b_nut", &ts_sec.to_string());
        self.set_bili_cookie("_uuid", &generate_guest_uuid(now));
        self.set_bili_cookie("b_lsid", &format!("{:08X}_{:X}", now as u32, now));

        // 1. 取得前端指紋 buvid3 / buvid4（`/x/frontend/finger/spi` 由回應 JSON 提供而非 Set-Cookie）
        let spi_resp = self
            .http
            .get(format!("{API}/x/frontend/finger/spi"))
            .send()
            .await;
        if let Ok(resp) = spi_resp
            && resp.status().is_success()
            && let Ok(bytes) = bounded_body(resp, 64 * 1024).await
            && let Ok(spi) = serde_json::from_slice::<Value>(&bytes)
            && spi["code"] == 0
        {
            self.set_bili_cookie("buvid3", &string(&spi["data"]["b_3"]));
            self.set_bili_cookie("buvid4", &string(&spi["data"]["b_4"]));
        }

        // 2. 簽發訪客 bili_ticket
        let hexsign = wbi::hmac_sha256_hex(b"XgwSnGZ1p", format!("ts{ts_sec}").as_bytes());
        let ticket_url = format!(
            "{API}/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket?key_id=ec02&hexsign={hexsign}&context[ts]={ts_sec}&csrf="
        );
        let ticket_resp = self.http.post(&ticket_url).send().await;
        if let Ok(resp) = ticket_resp
            && resp.status().is_success()
            && let Ok(bytes) = bounded_body(resp, 128 * 1024).await
            && let Ok(ticket_json) = serde_json::from_slice::<Value>(&bytes)
            && ticket_json["code"] == 0
        {
            let ticket = string(&ticket_json["data"]["ticket"]);
            self.set_bili_cookie("bili_ticket", &ticket);
            let created = number(&ticket_json["data"]["created_at"]);
            let ttl = number(&ticket_json["data"]["ttl"]);
            if created > 0 && ttl > 0 {
                self.set_bili_cookie("bili_ticket_expires", &(created + ttl).to_string());
            }
        }

        // 3. 啟用訪客 buvid3（避免 `feed/space` 進入未啟用靜默空包模式）
        let wuzhi_payload = json!({
            "payload": r#"{"3064":1,"39c8":"333.999.fp.risk","3c43":{"adca":"Win32","bfe9":"Mozilla/5.0"}}"#
        });
        let _ = self
            .http
            .post(format!("{API}/x/internal/gaia-gateway/ExClimbWuzhi"))
            .header(header::REFERER, "https://space.bilibili.com/")
            .header(header::ORIGIN, SPACE_ORIGIN)
            .json(&wuzhi_payload)
            .send()
            .await;

        *fetched = Some(Instant::now());
        Ok(())
    }

    async fn json(
        &self,
        url: &str,
        body: Option<Value>,
        ttl: Duration,
        fresh: bool,
    ) -> Result<Value> {
        self.json_with_referer(url, body, ttl, fresh, None).await
    }

    async fn json_with_referer(
        &self,
        url: &str,
        body: Option<Value>,
        ttl: Duration,
        fresh: bool,
        referer: Option<(&str, &str)>,
    ) -> Result<Value> {
        let cache_key = cache_key(url);
        if body.is_none()
            && !fresh
            && let Some((expires, value)) = self.cache.lock().await.get(&cache_key).cloned()
            && expires > Instant::now()
        {
            return Ok(value);
        }
        if self.bootstrap().await.is_err() {
            eprintln!("[Bili API] 訪客 Cookie 暫不可用，嘗試公開 API");
        }
        for attempt in 0..=1 {
            if referer.is_some() {
                self.pace_space_wbi().await;
            }
            let result: Result<Value> = async {
                let _permit = self.gate.acquire().await?;
                let mut request = self.http.request(
                    if body.is_some() {
                        Method::POST
                    } else {
                        Method::GET
                    },
                    url,
                );
                if let Some((ref_url, origin)) = referer {
                    request = request
                        .header(header::REFERER, ref_url)
                        .header(header::ORIGIN, origin);
                }
                if let Some(body) = &body {
                    request = request.json(body);
                }
                let response = request.send().await.context("B 站連線失敗或逾時")?;
                let status = response.status();
                if !status.is_success() {
                    bail!("B 站 HTTP {}", status.as_u16());
                }
                let bytes = bounded_body(response, 16 * 1024 * 1024).await?;
                serde_json::from_slice(&bytes).context("B 站回傳了無效 JSON")
            }
            .await;
            match result {
                Ok(value) => {
                    let code = value["code"].as_i64().unwrap_or(-1);
                    if matches!(code, -352 | -412 | -509 | -799) {
                        *self.visitor.lock().await = None;
                        if attempt == 0 {
                            tokio::time::sleep(Duration::from_millis(650 + now_ms() % 300)).await;
                            let _ = self.bootstrap().await;
                            continue;
                        }
                        if code != -352 {
                            bail!("B 站風控攔截（code {code}），請稍後重試");
                        }
                    }
                    if body.is_none() && code == 0 && !ttl.is_zero() {
                        self.cache
                            .lock()
                            .await
                            .put(cache_key.clone(), (Instant::now() + ttl, value.clone()));
                    }
                    return Ok(value);
                }
                Err(error) if attempt == 0 => {
                    let message = error.to_string();
                    if message.contains("HTTP 412") {
                        *self.visitor.lock().await = None;
                        tokio::time::sleep(Duration::from_millis(900 + now_ms() % 300)).await;
                        let _ = self.bootstrap().await;
                        continue;
                    }
                    if message.contains("HTTP 4") && !message.contains("429") {
                        return Err(error);
                    }
                    tokio::time::sleep(Duration::from_millis(800 + now_ms() % 300)).await;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }

    async fn signed_query(&self, params: BTreeMap<String, String>) -> Result<String> {
        let needs_fetch = self
            .keys
            .lock()
            .await
            .as_ref()
            .is_none_or(|keys| keys.fetched.elapsed() >= Duration::from_secs(12 * 3600));
        if needs_fetch {
            let nav = self
                .json(
                    &format!("{API}/x/web-interface/nav"),
                    None,
                    Duration::ZERO,
                    true,
                )
                .await?;
            let img = key_from_url(&string(&nav["data"]["wbi_img"]["img_url"]))?;
            let sub = key_from_url(&string(&nav["data"]["wbi_img"]["sub_url"]))?;
            *self.keys.lock().await = Some(WbiKeys {
                img,
                sub,
                fetched: Instant::now(),
            });
        }
        let keys_guard = self.keys.lock().await;
        let keys = keys_guard.as_ref().context("B 站未提供有效 WBI Key")?;
        Ok(wbi::sign(
            &params,
            &keys.img,
            &keys.sub,
            (now_ms() + 500) / 1000,
        ))
    }

    pub async fn resolve_up(&self, input: &str) -> Result<u64> {
        let mut input = input.trim().to_owned();
        ensure!(
            !input.is_empty() && input.len() <= 2048,
            "請輸入 UID、空間網址、b23.tv 短鏈或 BV 號"
        );
        for _ in 0..=1 {
            if input.bytes().all(|c| c.is_ascii_digit()) {
                return valid_uid(input.parse()?);
            }
            if input.starts_with("b23.tv/") || input.starts_with("space.bilibili.com/") {
                input = format!("https://{input}");
            }
            if let Ok(url) = Url::parse(&input) {
                if url.host_str() == Some("space.bilibili.com") {
                    if let Some(mid) = url.path_segments().and_then(|mut parts| parts.next()) {
                        return valid_uid(mid.parse().context("空間 UID 無效")?);
                    }
                } else if url.host_str() == Some("b23.tv") {
                    ensure!(
                        matches!(url.scheme(), "https" | "http")
                            && url.username().is_empty()
                            && url.password().is_none(),
                        "短鏈格式無效"
                    );
                    let mut url = url;
                    url.set_scheme("https")
                        .map_err(|_| anyhow!("短鏈格式無效"))?;
                    let _permit = self.gate.acquire().await?;
                    let response = self
                        .http
                        .get(url)
                        .send()
                        .await
                        .context("無法解析 b23.tv 短網址")?;
                    input = response.url().to_string();
                    continue;
                }
            }
            if let Some(bvid) = find_bvid(&input) {
                let value = self
                    .json(
                        &format!("{API}/x/web-interface/view?bvid={bvid}"),
                        None,
                        Duration::from_secs(3600),
                        false,
                    )
                    .await?;
                api_data(&value)?;
                return valid_uid(number(&value["data"]["owner"]["mid"]));
            }
            break;
        }
        bail!("無法辨識輸入，請輸入 UID、UP 主空間網址或 BV 號")
    }

    pub async fn get_profile(&self, mid: u64) -> Result<UpUser> {
        valid_uid(mid)?;
        let card = self
            .json(
                &format!("{API}/x/web-interface/card?mid={mid}&photo=true"),
                None,
                Duration::from_secs(300),
                false,
            )
            .await;
        if let Ok(value) = card
            && value["code"] == 0
            && value["data"]["card"].is_object()
        {
            return parse_profile(&value, true);
        }
        let query = self
            .signed_query(params(&[("mid", mid.to_string())]))
            .await?;
        let value = self
            .json(
                &format!("{API}/x/space/wbi/acc/info?{query}"),
                None,
                Duration::from_secs(300),
                false,
            )
            .await?;
        api_data(&value)?;
        parse_profile(&value, false)
    }

    pub async fn raw_dynamics(&self, mid: u64, offset: &str, fresh: bool) -> Result<DynamicPage> {
        valid_uid(mid)?;
        ensure!(offset.len() <= 512, "動態游標過長");
        let space_referer = format!("https://space.bilibili.com/{mid}/dynamic");
        for attempt in 0..=1 {
            let mut parameters = params(&[
                ("host_mid", mid.to_string()),
                ("offset", offset.to_owned()),
                ("timezone_offset", "-480".into()),
                ("platform", "web".into()),
                ("features", DYNAMIC_FEATURES.into()),
                ("web_location", "333.1387".into()),
            ]);
            insert_dm_img_params(&mut parameters);
            let query = self.signed_query(parameters).await?;
            let request_url = format!("{API}/x/polymer/web-dynamic/v1/feed/space?{query}");
            let value = self
                .json_with_referer(
                    &request_url,
                    None,
                    Duration::from_secs(12),
                    fresh || attempt > 0,
                    Some((&space_referer, SPACE_ORIGIN)),
                )
                .await?;
            let data = api_data(&value)?;
            let items: Vec<DynamicItem> = data["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|item| parse_dynamic(item, mid))
                .collect();
            let has_voucher = data
                .get("v_voucher")
                .and_then(Value::as_str)
                .is_some_and(|v| !v.is_empty());
            if items.is_empty() && offset.is_empty() && has_voucher {
                self.cache.lock().await.pop(&cache_key(&request_url));
                *self.visitor.lock().await = None;
                if attempt == 0 {
                    tokio::time::sleep(Duration::from_millis(650 + now_ms() % 300)).await;
                    let _ = self.bootstrap().await;
                    continue;
                }
                bail!("B 站動態觸發訪客風控校驗 (v_voucher)");
            }
            if items.is_empty() && offset.is_empty() {
                self.cache.lock().await.pop(&cache_key(&request_url));
            }
            let next = string(&data["offset"]);
            return Ok(DynamicPage {
                items,
                has_more: truthy(&data["has_more"]) && !next.is_empty() && next != offset,
                offset: next,
                warning: None,
            });
        }
        unreachable!()
    }

    pub async fn get_dynamics(&self, mid: u64, offset: &str, fresh: bool) -> Result<DynamicPage> {
        match self.raw_dynamics(mid, offset, fresh).await {
            Ok(page) => return Ok(page),
            Err(error) if !offset.is_empty() => return Err(error),
            Err(_) => {}
        }
        let videos = self.get_videos(mid, 1, 10).await?;
        ensure!(!videos.list.is_empty(), "動態讀取失敗，影片備援也沒有資料");
        Ok(DynamicPage {
            items: videos
                .list
                .into_iter()
                .map(|video| DynamicItem {
                    id_str: format!("video_{}", video.bvid),
                    r#type: DynamicType::Video,
                    mid: video.mid,
                    author_name: video.author,
                    author_face: video.author_face.unwrap_or_default(),
                    pub_time: video.pubdate.saturating_mul(1000),
                    text: Some(video.description),
                    title: Some(video.title),
                    cover: Some(video.pic),
                    jump_url: Some(format!("https://www.bilibili.com/video/{}", video.bvid)),
                    bvid: Some(video.bvid),
                    comment_id: (video.aid > 0).then(|| video.aid.to_string()),
                    comment_type: (video.aid > 0).then_some(1),
                    ..Default::default()
                })
                .collect(),
            warning: Some("動態 API 暫不可用，目前僅顯示影片備援資料".into()),
            ..Default::default()
        })
    }

    pub async fn get_videos(&self, mid: u64, page: u32, page_size: u32) -> Result<VideoPage> {
        valid_uid(mid)?;
        ensure!((1..=200).contains(&page), "影片頁碼須為 1–200");
        let size = page_size.clamp(1, 50);
        let mut search_params = params(&[
            ("mid", mid.to_string()),
            ("pn", page.to_string()),
            ("ps", size.to_string()),
            ("tid", "0".into()),
            ("order", "pubdate".into()),
            ("keyword", String::new()),
            ("order_avoided", "true".into()),
            ("platform", "web".into()),
            ("web_location", "1550101".into()),
        ]);
        insert_dm_img_params(&mut search_params);
        let video_referer = format!("https://space.bilibili.com/{mid}/video");
        if let Ok(query) = self.signed_query(search_params).await
            && let Ok(value) = self
                .json_with_referer(
                    &format!("{API}/x/space/wbi/arc/search?{query}"),
                    None,
                    Duration::from_secs(120),
                    false,
                    Some((&video_referer, SPACE_ORIGIN)),
                )
                .await
            && value["code"] == 0
            && let Some(list) = value["data"]["list"]["vlist"].as_array()
        {
            let videos = list
                .iter()
                .map(|v| parse::parse_video(v, mid, false))
                .collect::<Vec<_>>();
            let total = number(&value["data"]["page"]["count"]);
            return Ok(VideoPage {
                has_more: u64::from(page) * u64::from(size) < total,
                list: videos,
                total,
                ..Default::default()
            });
        }
        let media_url =
            format!("{API}/x/v2/medialist/resource/list?type=1&biz_id={mid}&ps={size}&pn={page}");
        if let Ok(value) = self
            .json(&media_url, None, Duration::from_secs(120), false)
            .await
            && value["code"] == 0
            && let Some(list) = value["data"]["media_list"].as_array()
        {
            let videos = list
                .iter()
                .map(|v| parse::parse_video(v, mid, true))
                .collect::<Vec<_>>();
            let total = value["data"]["total_count"]
                .as_u64()
                .unwrap_or(videos.len() as u64);
            return Ok(VideoPage {
                has_more: u64::from(page) * u64::from(size) < total,
                list: videos,
                total,
                warning: None,
                ..Default::default()
            });
        }
        let end = (page * size) as usize;
        let mut videos = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = String::new();
        let mut more = true;
        for _ in 0..100 {
            let result = self
                .raw_dynamics(mid, &offset, false)
                .await
                .context("投稿與動態備援 API 皆不可用")?;
            for item in result.items {
                if item.r#type == DynamicType::Video
                    && let Some(bvid) = &item.bvid
                    && seen.insert(bvid.clone())
                {
                    videos.push(item);
                }
            }
            more = result.has_more;
            offset = result.offset;
            if !more || videos.len() > end {
                break;
            }
            ensure!(
                videos.len() < 2000,
                "影片備援搜尋超過安全範圍，請選擇較近的頁數"
            );
        }
        let total = videos.len().max(if more { end + 1 } else { 0 }) as u64;
        more = more || videos.len() > end;
        let list = videos
            .into_iter()
            .skip(((page - 1) * size) as usize)
            .take(size as usize)
            .map(|item| VideoItem {
                bvid: item.bvid.unwrap_or_default(),
                title: item.title.unwrap_or_default(),
                description: item.text.unwrap_or_default(),
                pic: item.cover.unwrap_or_default(),
                pubdate: item.pub_time / 1000,
                created: item.pub_time / 1000,
                mid: item.mid,
                author: item.author_name,
                author_face: Some(item.author_face),
                ..Default::default()
            })
            .collect();
        Ok(VideoPage {
            list,
            total,
            has_more: more,
            total_estimated: more,
            warning: Some("目前從動態中提取影片，總數為估計值".into()),
        })
    }

    pub async fn get_live(&self, uids: &[u64]) -> Result<BTreeMap<u64, LiveStatusInfo>> {
        let mut result = BTreeMap::new();
        for batch in uids.chunks(100) {
            let value = self
                .json(
                    &format!("{LIVE_API}/room/v1/Room/get_status_info_by_uids"),
                    Some(json!({"uids": batch})),
                    Duration::ZERO,
                    true,
                )
                .await?;
            let data = api_data(&value)?;
            // B 站可能用 [] 表示空結果；缺少 UID 留給 Poller 保留原有狀態。
            if data.is_array() && data.as_array().is_some_and(Vec::is_empty) {
                continue;
            }
            let map = data.as_object().context("直播 API 回應格式無效")?;
            for (uid, item) in map {
                let uid: u64 = uid.parse().context("直播 API UID 無效")?;
                if !batch.contains(&uid) {
                    continue;
                }
                result.insert(
                    uid,
                    LiveStatusInfo {
                        uid,
                        room_id: number(&item["room_id"]),
                        title: string(&item["title"]),
                        cover_from_user: url_string(&item["cover_from_user"]),
                        keyframe: url_string(&item["keyframe"]),
                        live_status: number(&item["live_status"]) as u32,
                        live_time: string(&item["live_time"]),
                        online: number(&item["online"]),
                        area_name: string(item.get("area_v2_name").unwrap_or(&item["area_name"])),
                        parent_area_name: string(
                            item.get("area_v2_parent_name")
                                .unwrap_or(&item["parent_area_name"]),
                        ),
                        uname: item["uname"].as_str().map(str::to_owned),
                        face: item["face"].as_str().map(normalize_url),
                    },
                );
            }
        }
        Ok(result)
    }

    pub async fn get_live_room(&self, id: u64) -> Result<crate::live_watch::ObservedRoom> {
        ensure!(crate::live_watch::valid_id(id), "直播房號無效");
        for attempt in 0..2 {
            let _permit = self.gate.acquire().await?;
            let response = self
                .http
                .get(format!(
                    "{LIVE_API}/xlive/web-room/v1/index/getRoomBaseInfo"
                ))
                .query(&[
                    ("req_biz", "video".to_owned()),
                    ("room_ids", id.to_string()),
                ])
                .header(header::REFERER, format!("https://live.bilibili.com/{id}"))
                .timeout(Duration::from_secs(8))
                .send()
                .await
                .context("直播 API 連線失敗或逾時")?;
            if (response.status().as_u16() == 429 || response.status().is_server_error())
                && attempt == 0
            {
                drop(_permit);
                tokio::time::sleep(Duration::from_millis(500)).await;
                continue;
            }
            ensure!(
                response.status().is_success(),
                "直播 API HTTP {}",
                response.status()
            );
            let bytes = bounded_body(response, 2 * 1024 * 1024).await?;
            let value: Value = serde_json::from_slice(&bytes).context("直播 API JSON 無效")?;
            let _ = api_data(&value)?;
            return crate::live_watch::normalize_info(id, &value, now_ms());
        }
        bail!("直播 API 重試失敗")
    }

    pub async fn get_comments(
        &self,
        kind: u32,
        oid: &str,
        page: u32,
        page_size: u32,
    ) -> Result<CommentsResult> {
        ensure!(
            !oid.is_empty()
                && oid.len() <= 64
                && oid.bytes().all(|c| c.is_ascii_digit())
                && kind > 0,
            "此動態缺少有效評論區參數"
        );
        let size = page_size.clamp(1, 20);
        let page = page.max(1);
        let value = self
            .json(
                &format!("{API}/x/v2/reply?type={kind}&oid={oid}&pn={page}&ps={size}&sort=2"),
                None,
                Duration::from_secs(30),
                false,
            )
            .await?;
        if value["code"] != 0 {
            let code = value["code"].as_i64().unwrap_or(-1);
            let reason = match code {
                -400 => "評論參數不受支援",
                -404 | 12002 => "此內容的評論區已關閉",
                -352 | -412 | 412 => "B 站風控限制，請稍後重試",
                _ => "評論 API 暫不可用",
            };
            return Ok(CommentsResult {
                error: Some(format!("{reason}（code {code}）")),
                ..Default::default()
            });
        }
        let data = api_data(&value)?;
        let total = number(&data["page"]["count"]);
        let list = data["replies"]
            .as_array()
            .into_iter()
            .flatten()
            .map(parse_comment)
            .collect();
        Ok(CommentsResult {
            list,
            total,
            has_more: u64::from(page) * u64::from(size) < total,
            error: None,
        })
    }
}

impl BiliApi for ApiClient {
    fn room(&self, id: u64) -> BoxFuture<'_, Result<crate::live_watch::ObservedRoom>> {
        Box::pin(self.get_live_room(id))
    }
    fn live(&self, uids: Vec<u64>) -> BoxFuture<'_, Result<BTreeMap<u64, LiveStatusInfo>>> {
        Box::pin(async move { self.get_live(&uids).await })
    }
    fn dynamics(&self, mid: u64, offset: String) -> BoxFuture<'_, Result<DynamicPage>> {
        Box::pin(async move { self.get_dynamics(mid, &offset, false).await })
    }
}

fn insert_dm_img_params(parameters: &mut BTreeMap<String, String>) {
    parameters.insert("dm_img_list".into(), "[]".into());
    parameters.insert("dm_img_str".into(), DM_IMG_STR.into());
    parameters.insert("dm_cover_img_str".into(), DM_COVER_IMG_STR.into());
    parameters.insert("dm_img_inter".into(), DM_IMG_INTER.into());
}

fn generate_guest_uuid(seed_ms: u64) -> String {
    let d1 = md5::compute(format!("bili-uuid-a-{seed_ms}").as_bytes());
    let d2 = md5::compute(format!("bili-uuid-b-{}", seed_ms.wrapping_mul(31)).as_bytes());
    let b = d1.0;
    let c = d2.0;
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-4{:01X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:05}infoc",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6] & 0x0f,
        b[7],
        (b[8] & 0x3f) | 0x80,
        b[9],
        c[0],
        c[1],
        c[2],
        c[3],
        c[4],
        c[5],
        seed_ms % 100_000
    )
}

pub fn normalize_url(url: &str) -> String {
    if url.starts_with("//") {
        format!("https:{url}")
    } else if let Some(tail) = url.strip_prefix("http://") {
        format!("https://{tail}")
    } else {
        url.into()
    }
}

pub fn allowed_bili_host(host: &str) -> bool {
    ["bilibili.com", "b23.tv", "hdslb.com", "biliimg.com"]
        .iter()
        .any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
}

pub(crate) async fn bounded_body(
    mut response: reqwest::Response,
    maximum: usize,
) -> Result<Vec<u8>> {
    ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= maximum as u64),
        "回應內容超過安全上限"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.context("下載內容失敗")? {
        ensure!(bytes.len() + chunk.len() <= maximum, "回應內容超過安全上限");
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn valid_uid(uid: u64) -> Result<u64> {
    ensure!(uid > 0 && uid <= MAX_SAFE_UID, "UID 必須是有效的正整數");
    Ok(uid)
}
fn params(items: &[(&str, String)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(key, value)| ((*key).into(), value.clone()))
        .collect()
}
fn api_data(value: &Value) -> Result<&Value> {
    ensure!(
        value["code"] == 0,
        "B 站 API 回應錯誤（code {}）",
        value["code"]
    );
    value.get("data").context("B 站 API 缺少 data")
}
fn truthy(value: &Value) -> bool {
    value.as_bool().unwrap_or_else(|| number(value) > 0)
}
fn key_from_url(value: &str) -> Result<String> {
    let url = Url::parse(value).context("WBI Key 網址無效")?;
    let key = url
        .path_segments()
        .and_then(|parts| parts.last())
        .and_then(|file| file.split('.').next())
        .unwrap_or_default();
    ensure!(
        key.len() == 32 && key.bytes().all(|c| c.is_ascii_hexdigit()),
        "B 站未提供有效 WBI Key"
    );
    Ok(key.to_owned())
}

pub fn find_bvid(input: &str) -> Option<String> {
    input
        .as_bytes()
        .windows(12)
        .find(|window| {
            window[..2].eq_ignore_ascii_case(b"BV")
                && window[2..].iter().all(u8::is_ascii_alphanumeric)
        })
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .map(str::to_owned)
}

fn cache_key(input: &str) -> String {
    let Ok(mut url) = Url::parse(input) else {
        return input.into();
    };
    let mut parameters = url
        .query_pairs()
        .filter(|(key, _)| key != "w_rid" && key != "wts")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parameters.sort();
    url.set_query(None);
    if !parameters.is_empty() {
        url.query_pairs_mut().extend_pairs(parameters);
    }
    url.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn mock(
        statuses: Vec<u16>,
    ) -> Result<(String, tokio::task::JoinHandle<Result<Vec<String>>>)> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/sample", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let mut headers = Vec::new();
            for status in statuses {
                let (mut socket, _) = listener.accept().await?;
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let length = socket.read(&mut buffer).await?;
                    if length == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..length]);
                    ensure!(bytes.len() < 16 * 1024, "mock request too large");
                    if bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                        break;
                    }
                }
                headers.push(String::from_utf8(bytes)?);
                let body = r#"{"code":0,"data":{"ok":true}}"#;
                let response = format!(
                    "HTTP/1.1 {status} Sample\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await?;
            }
            Ok(headers)
        });
        Ok((url, task))
    }

    #[tokio::test]
    async fn http_headers_and_ttl_cache_work_with_real_local_io() -> Result<()> {
        let (url, server) = mock(vec![200]).await?;
        let client = ApiClient::build(None, false)?;
        *client.visitor.lock().await = Some(Instant::now());
        assert_eq!(
            client
                .json(&url, None, Duration::from_secs(60), false)
                .await?["data"]["ok"],
            true
        );
        client
            .json(&url, None, Duration::from_secs(60), false)
            .await?;
        let requests = server.await??;
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .to_lowercase()
                .contains("referer: https://www.bilibili.com/")
        );
        assert!(requests[0].contains(USER_AGENT));
        Ok(())
    }

    #[tokio::test]
    async fn http_429_has_a_bounded_retry_and_wbi_timestamps_share_cache() -> Result<()> {
        let (url, server) = mock(vec![429, 200]).await?;
        let client = ApiClient::build(None, false)?;
        *client.visitor.lock().await = Some(Instant::now());
        client.json(&url, None, Duration::ZERO, false).await?;
        assert_eq!(server.await??.len(), 2);
        assert_eq!(
            cache_key("https://api.bilibili.com/feed?mid=1&wts=1&w_rid=a"),
            cache_key("https://api.bilibili.com/feed?w_rid=b&wts=2&mid=1")
        );
        Ok(())
    }
}
