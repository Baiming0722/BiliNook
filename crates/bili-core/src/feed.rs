use crate::{api::BiliApi, models::*, wbi::encode_component};
use anyhow::{Result, anyhow, ensure};
use futures::{StreamExt, stream};
use lru::LruCache;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Default)]
pub struct FeedQuery {
    pub mids: Vec<u64>,
    pub kind: Option<DynamicType>,
    pub keyword: String,
    pub cursor: Option<String>,
    pub limit: usize,
}
#[derive(Clone, Debug, Default)]
pub struct FeedPage {
    pub items: Vec<DynamicItem>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub total: usize,
    pub warning: Option<String>,
}
#[derive(Clone)]
struct Entry {
    items: Vec<DynamicItem>,
    offset: String,
    has_more: bool,
    fetched: Option<Instant>,
    warning: Option<String>,
    stale: bool,
    head_truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedCursor {
    pub time: u64,
    pub id: String,
}
impl FeedCursor {
    pub fn encode(&self) -> String {
        format!("v1:{}:{}", self.time, encode_component(&self.id))
    }
    pub fn parse(input: &str) -> Result<Self> {
        let mut parts = input.splitn(3, ':');
        ensure!(parts.next() == Some("v1"), "動態分頁游標無效");
        let time = parts
            .next()
            .ok_or_else(|| anyhow!("動態分頁游標無效"))?
            .parse()?;
        let id = percent_encoding::percent_decode_str(
            parts.next().ok_or_else(|| anyhow!("動態分頁游標無效"))?,
        )
        .decode_utf8()?
        .into_owned();
        ensure!(!id.is_empty() && id.len() <= 256, "動態分頁游標無效");
        Ok(Self { time, id })
    }
}

pub struct FeedService {
    api: Arc<dyn BiliApi>,
    cache: Mutex<LruCache<u64, Entry>>,
    flights: Mutex<HashMap<u64, Weak<Mutex<()>>>>,
}
impl FeedService {
    pub fn new(api: Arc<dyn BiliApi>) -> Self {
        Self {
            api,
            cache: Mutex::new(LruCache::new(NonZeroUsize::new(1000).unwrap())),
            flights: Mutex::new(HashMap::new()),
        }
    }
    pub async fn invalidate(&self) {
        for (_, entry) in self.cache.lock().await.iter_mut() {
            entry.fetched = None;
        }
    }

    pub async fn fetch(&self, query: FeedQuery, cancel: CancellationToken) -> Result<FeedPage> {
        tokio::select! {biased;_ = cancel.cancelled()=>Err(anyhow!("已取消動態載入")),result=self.query(query)=>result}
    }
    async fn load(
        &self,
        mid: u64,
        more: bool,
        cursor: Option<&FeedCursor>,
        previous: Option<Entry>,
    ) -> Result<Entry> {
        let flight = {
            let mut flights = self.flights.lock().await;
            if flights.len() > 1000 {
                flights.retain(|_, flight| flight.strong_count() > 0);
            }
            if let Some(flight) = flights.get(&mid).and_then(Weak::upgrade) {
                flight
            } else {
                let flight = Arc::new(Mutex::new(()));
                flights.insert(mid, Arc::downgrade(&flight));
                flight
            }
        };
        let _guard = flight.lock().await;
        let old = previous.or(self.cache.lock().await.get(&mid).cloned());
        if !more && let Some(entry) = &old {
            let newest = entry.items.iter().max_by(|a, b| {
                a.pub_time
                    .cmp(&b.pub_time)
                    .then_with(|| a.id_str.cmp(&b.id_str))
            });
            let needs_head = entry.head_truncated
                && newest.is_none_or(|newest| {
                    cursor.is_none_or(|cursor| {
                        cursor.time > newest.pub_time
                            || (cursor.time == newest.pub_time && cursor.id > newest.id_str)
                    })
                });
            let ttl = if entry.items.is_empty() { 5 } else { 90 };
            if !needs_head
                && entry
                    .fetched
                    .is_some_and(|time| time.elapsed() < Duration::from_secs(ttl))
            {
                return Ok(entry.clone());
            }
        }
        let offset = if more {
            old.as_ref()
                .map(|entry| entry.offset.clone())
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut page = match self.api.dynamics(mid, offset.clone()).await {
            Ok(page) => page,
            Err(_) if !more && old.as_ref().is_some_and(|entry| !entry.items.is_empty()) => {
                let mut stale = old.unwrap();
                stale.stale = true;
                stale.warning = Some(format!("UP {mid} 更新失敗，顯示先前動態"));
                return Ok(stale);
            }
            Err(error) => return Err(error),
        };
        if !more && page.items.is_empty() {
            tokio::time::sleep(Duration::from_millis(500)).await;
            match self.api.dynamics(mid, offset.clone()).await {
                Ok(retry) if !retry.items.is_empty() => page = retry,
                retry if old.as_ref().is_some_and(|entry| !entry.items.is_empty()) => {
                    let mut stale = old.unwrap();
                    stale.stale = true;
                    let detail = match retry {
                        Ok(_) => "連續回傳空動態".to_owned(),
                        Err(error) => format!("空動態後重試失敗：{error}"),
                    };
                    stale.warning = Some(format!(
                        "UP {mid} {detail}，暫時顯示先前動態；請稍後重新整理"
                    ));
                    return Ok(stale);
                }
                Ok(retry) => page = retry,
                Err(error) => return Err(error),
            }
        }
        ensure!(page.items.len() <= 2000, "單頁動態超過安全上限");
        let mut items = if more {
            old.as_ref()
                .map(|entry| entry.items.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut seen: HashSet<_> = items.iter().map(|item| item.id_str.clone()).collect();
        items.extend(
            page.items
                .into_iter()
                .filter(|item| seen.insert(item.id_str.clone())),
        );
        let mut truncated = more && old.as_ref().is_some_and(|entry| entry.head_truncated);
        if items.len() > 2000 {
            items.sort_by(compare);
            items.drain(..items.len() - 2000);
            truncated = true;
        }
        let entry = Entry {
            items,
            has_more: page.has_more && !page.offset.is_empty() && page.offset != offset,
            offset: page.offset,
            fetched: if more {
                old.as_ref().and_then(|entry| entry.fetched)
            } else {
                Some(Instant::now())
            },
            warning: page.warning,
            stale: false,
            head_truncated: truncated,
        };
        let mut cache = self.cache.lock().await;
        cache.put(mid, entry.clone());
        let mut count = cache
            .iter()
            .map(|(_, entry)| entry.items.len())
            .sum::<usize>();
        while count > 20000 {
            if let Some((_, old)) = cache.pop_lru() {
                count -= old.items.len();
            } else {
                break;
            }
        }
        Ok(entry)
    }
    async fn query(&self, query: FeedQuery) -> Result<FeedPage> {
        let mut mids = query.mids;
        mids.sort_unstable();
        mids.dedup();
        ensure!(mids.len() <= 1000, "單次最多查詢 1000 位 UP 主");
        if mids.is_empty() {
            return Ok(FeedPage::default());
        }
        let cursor = query.cursor.as_deref().map(FeedCursor::parse).transpose()?;
        let limit = if query.limit == 0 {
            20
        } else {
            query.limit.clamp(1, 100)
        };
        let keyword = query.keyword.trim().to_lowercase();
        let results = stream::iter(mids)
            .map(|mid| {
                let cursor = cursor.clone();
                async move { (mid, self.load(mid, false, cursor.as_ref(), None).await) }
            })
            .buffer_unordered(4)
            .collect::<Vec<_>>()
            .await;
        let mut entries = Vec::new();
        let mut warnings = Vec::new();
        let mut incomplete = false;
        for (mid, result) in results {
            match result {
                Ok(entry) => {
                    incomplete |= entry.stale || entry.warning.is_some();
                    entries.push((mid, entry));
                }
                Err(error) => {
                    incomplete = true;
                    warnings.push(format!("UP {mid}：{error}"));
                }
            }
        }
        ensure!(
            !entries.is_empty(),
            "所有動態載入失敗：{}",
            warnings
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("；")
        );
        let mut pages = 0;
        let filtered = loop {
            ensure!(
                entries
                    .iter()
                    .map(|(_, entry)| entry.items.len())
                    .sum::<usize>()
                    <= 20000,
                "動態超過安全範圍，請縮小分組"
            );
            let mut all = entries
                .iter()
                .flat_map(|(_, entry)| entry.items.iter().cloned())
                .collect::<Vec<_>>();
            all.sort_by(compare);
            let mut seen = HashSet::new();
            all.retain(|item| seen.insert(item.id_str.clone()));
            all.retain(|item| {
                query.kind.is_none_or(|kind| kind == item.r#type)
                    && (keyword.is_empty()
                        || [
                            item.text.as_deref(),
                            item.title.as_deref(),
                            Some(&item.author_name),
                        ]
                        .into_iter()
                        .flatten()
                        .any(|text| text.to_lowercase().contains(&keyword)))
                    && cursor.as_ref().is_none_or(|cursor| {
                        item.pub_time < cursor.time
                            || (item.pub_time == cursor.time && item.id_str < cursor.id)
                    })
            });
            let boundary = all.get(limit - 1);
            let indices = entries
                .iter()
                .enumerate()
                .filter(|(_, (_, entry))| {
                    entry.has_more
                        && !entry.stale
                        && boundary.is_none_or(|boundary| {
                            entry
                                .items
                                .iter()
                                .min_by(|a, b| compare(a, b).reverse())
                                .is_none_or(|oldest| compare(oldest, boundary).is_le())
                        })
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if indices.is_empty() {
                break all;
            }
            pages += indices.len();
            ensure!(pages <= 200, "搜尋範圍過大，請縮小分組或關鍵字");
            let work: Vec<_> = indices
                .into_iter()
                .map(|index| (index, entries[index].0, entries[index].1.clone()))
                .collect();
            let results = stream::iter(work)
                .map(|(index, mid, previous)| async move {
                    (index, self.load(mid, true, None, Some(previous)).await)
                })
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await;
            for (index, result) in results {
                match result {
                    Ok(entry) => entries[index].1 = entry,
                    Err(error) => {
                        let (mid, entry) = &mut entries[index];
                        entry.stale = true;
                        incomplete = true;
                        warnings.push(format!(
                            "UP {mid} 載入較舊動態失敗：{error}；目前顯示已取得的內容，請重新載入"
                        ));
                    }
                }
            }
        };
        warnings.extend(
            entries
                .iter()
                .filter_map(|(_, entry)| entry.warning.clone()),
        );
        if incomplete {
            warnings.push("動態尚未完整載入，請重新載入後再繼續分頁".into());
        }
        warnings.sort();
        warnings.dedup();
        let total = filtered.len();
        // 無法確定遺失分頁的排序邊界時，不產生可能跳過動態的游標。
        let has_more =
            !incomplete && (total > limit || entries.iter().any(|(_, entry)| entry.has_more));
        let items = filtered.into_iter().take(limit).collect::<Vec<_>>();
        let next_cursor = if has_more {
            items.last().map(|item| {
                FeedCursor {
                    time: item.pub_time,
                    id: item.id_str.clone(),
                }
                .encode()
            })
        } else {
            None
        };
        Ok(FeedPage {
            items,
            next_cursor,
            has_more: has_more && total > 0,
            total,
            warning: (!warnings.is_empty()).then(|| warnings.join("；")),
        })
    }
}
fn compare(a: &DynamicItem, b: &DynamicItem) -> std::cmp::Ordering {
    b.pub_time
        .cmp(&a.pub_time)
        .then_with(|| b.id_str.cmp(&a.id_str))
}
