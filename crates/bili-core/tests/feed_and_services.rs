use anyhow::{Result, anyhow};
use bili_core::{
    api::BiliApi,
    feed::{FeedCursor, FeedQuery, FeedService},
    models::*,
    services::{Services, UpPatch},
};
use futures::future::BoxFuture;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

struct SampleApi {
    pages: BTreeMap<(u64, String), DynamicPage>,
    calls: Mutex<Vec<(u64, String)>>,
    fail: Mutex<bool>,
}

struct EmptyHeadApi {
    calls: AtomicUsize,
    recover_on_retry: bool,
    empty_first: bool,
}

struct PartialHeadApi {
    calls: AtomicUsize,
}

impl BiliApi for PartialHeadApi {
    fn live(&self, _: Vec<u64>) -> BoxFuture<'_, Result<BTreeMap<u64, LiveStatusInfo>>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }
    fn dynamics(&self, mid: u64, _: String) -> BoxFuture<'_, Result<DynamicPage>> {
        Box::pin(async move {
            if mid == 2 && self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(anyhow!("暫時連線失敗"));
            }
            Ok(page(
                if mid == 1 {
                    vec![item("a", 80, 1), item("b", 70, 1)]
                } else {
                    vec![item("c", 100, 2), item("d", 90, 2)]
                },
                "",
            ))
        })
    }
}

#[tokio::test]
async fn failed_creator_head_blocks_cursor_until_reload() -> Result<()> {
    let service = FeedService::new(Arc::new(PartialHeadApi {
        calls: AtomicUsize::new(0),
    }));
    let query = FeedQuery {
        mids: vec![1, 2],
        limit: 1,
        ..Default::default()
    };
    let first = service
        .fetch(query.clone(), CancellationToken::new())
        .await?;
    assert_eq!(first.items[0].id_str, "a");
    assert!(first.warning.is_some());
    assert!(!first.has_more);
    assert!(first.next_cursor.is_none());
    let recovered = service.fetch(query, CancellationToken::new()).await?;
    assert_eq!(recovered.items[0].id_str, "c");
    assert!(recovered.has_more);
    assert!(recovered.next_cursor.is_some());
    Ok(())
}
impl BiliApi for EmptyHeadApi {
    fn live(&self, _: Vec<u64>) -> BoxFuture<'_, Result<BTreeMap<u64, LiveStatusInfo>>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }
    fn dynamics(&self, mid: u64, _: String) -> BoxFuture<'_, Result<DynamicPage>> {
        Box::pin(async move {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            let items = if self.empty_first && call == 0 {
                vec![]
            } else if self.empty_first {
                vec![item("new", 200, mid)]
            } else if call == 0 {
                vec![item("old", 100, mid)]
            } else if call >= 2 && self.recover_on_retry {
                vec![item("new", 200, mid)]
            } else {
                vec![]
            };
            Ok(page(items, ""))
        })
    }
}

#[tokio::test]
async fn empty_head_retry_recovers_without_discarding_group_dynamics() -> Result<()> {
    let api = Arc::new(EmptyHeadApi {
        calls: AtomicUsize::new(0),
        recover_on_retry: true,
        empty_first: false,
    });
    let service = FeedService::new(api.clone());
    let query = FeedQuery {
        mids: vec![1],
        ..Default::default()
    };
    assert_eq!(
        service
            .fetch(query.clone(), CancellationToken::new())
            .await?
            .items[0]
            .id_str,
        "old"
    );
    service.invalidate().await;
    let result = service.fetch(query, CancellationToken::new()).await?;
    assert_eq!(result.items[0].id_str, "new");
    assert!(result.warning.is_none());
    assert_eq!(api.calls.load(Ordering::SeqCst), 3);
    Ok(())
}

#[tokio::test]
async fn empty_head_after_retry_keeps_previous_with_warning() -> Result<()> {
    let api = Arc::new(EmptyHeadApi {
        calls: AtomicUsize::new(0),
        recover_on_retry: false,
        empty_first: false,
    });
    let service = FeedService::new(api.clone());
    let query = FeedQuery {
        mids: vec![1],
        ..Default::default()
    };
    service
        .fetch(query.clone(), CancellationToken::new())
        .await?;
    service.invalidate().await;
    let result = service.fetch(query, CancellationToken::new()).await?;
    assert_eq!(result.items[0].id_str, "old");
    assert!(
        result
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("空"))
    );
    assert_eq!(api.calls.load(Ordering::SeqCst), 3);
    Ok(())
}

#[tokio::test]
async fn initial_empty_head_retries_before_showing_an_empty_group() -> Result<()> {
    let api = Arc::new(EmptyHeadApi {
        calls: AtomicUsize::new(0),
        recover_on_retry: true,
        empty_first: true,
    });
    let result = FeedService::new(api.clone())
        .fetch(
            FeedQuery {
                mids: vec![1],
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(result.items[0].id_str, "new");
    assert_eq!(api.calls.load(Ordering::SeqCst), 2);
    Ok(())
}
impl BiliApi for SampleApi {
    fn live(&self, _: Vec<u64>) -> BoxFuture<'_, Result<BTreeMap<u64, LiveStatusInfo>>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }
    fn dynamics(&self, mid: u64, offset: String) -> BoxFuture<'_, Result<DynamicPage>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((mid, offset.clone()));
            if *self.fail.lock().unwrap() {
                return Err(anyhow!("樣本連線中斷"));
            }
            self.pages
                .get(&(mid, offset))
                .cloned()
                .ok_or_else(|| anyhow!("沒有樣本分頁"))
        })
    }
}
fn item(id: &str, time: u64, mid: u64) -> DynamicItem {
    DynamicItem {
        id_str: id.into(),
        pub_time: time,
        mid,
        r#type: DynamicType::Text,
        ..Default::default()
    }
}
fn page(items: Vec<DynamicItem>, offset: &str) -> DynamicPage {
    DynamicPage {
        items,
        offset: offset.into(),
        has_more: !offset.is_empty(),
        warning: None,
    }
}
fn api() -> Arc<SampleApi> {
    Arc::new(SampleApi {
        pages: BTreeMap::from([
            (
                (1, String::new()),
                page(vec![item("a", 300, 1), item("b", 100, 1)], "older"),
            ),
            ((1, "older".into()), page(vec![item("f", 80, 1)], "")),
            (
                (2, String::new()),
                page(vec![item("c", 250, 2), item("d", 240, 2)], "older"),
            ),
            ((2, "older".into()), page(vec![item("e", 230, 2)], "")),
        ]),
        calls: Mutex::new(Vec::new()),
        fail: Mutex::new(false),
    })
}

#[tokio::test]
async fn failed_older_page_keeps_other_creators_visible() -> Result<()> {
    let api = Arc::new(SampleApi {
        pages: BTreeMap::from([
            ((1, String::new()), page(vec![item("a", 300, 1)], "missing")),
            ((2, String::new()), page(vec![item("b", 250, 2)], "")),
        ]),
        calls: Mutex::new(Vec::new()),
        fail: Mutex::new(false),
    });
    let result = FeedService::new(api.clone())
        .fetch(
            FeedQuery {
                mids: vec![1, 2],
                limit: 10,
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| item.id_str.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(
        result
            .warning
            .as_deref()
            .is_some_and(|warning| warning.contains("較舊動態"))
    );
    assert_eq!(api.calls.lock().unwrap().len(), 3);
    Ok(())
}

#[test]
fn cursor_is_stable_and_rejects_invalid_format() -> Result<()> {
    let cursor = FeedCursor {
        time: 123,
        id: "id:/中文".into(),
    };
    assert_eq!(FeedCursor::parse(&cursor.encode())?, cursor);
    for input in ["garbage", "v1:no:1", "v1:1:", "v2:1:x"] {
        assert!(FeedCursor::parse(input).is_err());
    }
    Ok(())
}
#[tokio::test]
async fn aggregate_cursor_never_skips_hidden_creator_pages() -> Result<()> {
    let api = api();
    let service = FeedService::new(api.clone());
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let page = service
            .fetch(
                FeedQuery {
                    mids: vec![1, 2],
                    limit: 2,
                    cursor,
                    ..Default::default()
                },
                CancellationToken::new(),
            )
            .await?;
        ids.extend(page.items.iter().map(|item| item.id_str.clone()));
        cursor = page.next_cursor;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(ids, vec!["a", "c", "d", "e", "b", "f"]);
    assert_eq!(api.calls.lock().unwrap().len(), 4);
    Ok(())
}
#[tokio::test]
async fn keyword_search_loads_older_pages_until_it_finds_a_match() -> Result<()> {
    let mut api = SampleApi {
        pages: BTreeMap::new(),
        calls: Mutex::new(Vec::new()),
        fail: Mutex::new(false),
    };
    api.pages
        .insert((1, String::new()), page(vec![item("1", 200, 1)], "next"));
    let mut target = item("2", 100, 1);
    target.text = Some("繁體 關鍵字".into());
    api.pages.insert((1, "next".into()), page(vec![target], ""));
    let service = FeedService::new(Arc::new(api));
    let result = service
        .fetch(
            FeedQuery {
                mids: vec![1],
                keyword: "關鍵字".into(),
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(result.items[0].id_str, "2");
    assert!(!result.has_more);
    Ok(())
}
#[tokio::test]
async fn equal_timestamps_use_id_to_avoid_duplicates() -> Result<()> {
    let mut api = SampleApi {
        pages: BTreeMap::new(),
        calls: Mutex::new(Vec::new()),
        fail: Mutex::new(false),
    };
    api.pages.insert(
        (1, String::new()),
        page(vec![item("30", 100, 1), item("10", 100, 1)], "next"),
    );
    api.pages
        .insert((1, "next".into()), page(vec![item("20", 100, 1)], ""));
    let service = FeedService::new(Arc::new(api));
    let first = service
        .fetch(
            FeedQuery {
                mids: vec![1],
                limit: 2,
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(
        first
            .items
            .iter()
            .map(|i| i.id_str.as_str())
            .collect::<Vec<_>>(),
        vec!["30", "20"]
    );
    let second = service
        .fetch(
            FeedQuery {
                mids: vec![1],
                limit: 2,
                cursor: first.next_cursor,
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(second.items[0].id_str, "10");
    Ok(())
}
#[tokio::test]
async fn stale_head_is_visible_with_warning_and_cancelled_requests_do_no_io() -> Result<()> {
    let api = api();
    let service = FeedService::new(api.clone());
    service
        .fetch(
            FeedQuery {
                mids: vec![1],
                limit: 1,
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await?;
    *api.fail.lock().unwrap() = true;
    service.invalidate().await;
    let page = service
        .fetch(
            FeedQuery {
                mids: vec![1],
                limit: 1,
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(page.items[0].id_str, "a");
    assert!(page.warning.is_some());
    let calls = api.calls.lock().unwrap().len();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        service
            .fetch(
                FeedQuery {
                    mids: vec![1],
                    ..Default::default()
                },
                cancel
            )
            .await
            .is_err()
    );
    assert_eq!(api.calls.lock().unwrap().len(), calls);
    Ok(())
}
#[tokio::test]
async fn settings_patch_does_not_overwrite_recent_group_changes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let services = Services::open(dir.path().into()).await?;
    services
        .save_groups(vec![FollowGroup {
            id: "group-a".into(),
            name: "新分組".into(),
            mids: vec![1],
            ..Default::default()
        }])
        .await?;
    services.patch_settings(json!({"notifyLive":false})).await?;
    let snapshot = services.snapshot().await?;
    assert_eq!(snapshot.settings.follow_groups.unwrap()[0].name, "新分組");
    assert!(!snapshot.settings.notify_live);
    assert!(
        services
            .patch_settings(json!({"followGroups":[]}))
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn failed_shutdown_can_resume_without_reopening_the_store() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let services = Services::open(dir.path().into()).await?;
    let mut events = services.poller.subscribe();
    std::fs::create_dir(dir.path().join("ups.json"))?;
    assert!(
        services
            .store
            .update(|s| {
                s.ups.push(UpUser {
                    mid: 7,
                    ..Default::default()
                });
                Ok(())
            })
            .await
            .is_err()
    );
    assert!(services.shutdown().await.is_err());
    let resumed = services.resume_after_shutdown().await?;
    assert!(!resumed.stop.is_cancelled());
    assert!(Arc::ptr_eq(&services.poller, &resumed.poller));
    assert!(services.resume_after_shutdown().await.is_err());
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if matches!(
                events.recv().await?,
                bili_core::poller::CoreEvent::Polling { active: true, .. }
            ) {
                return Ok::<_, tokio::sync::broadcast::error::RecvError>(());
            }
        }
    })
    .await??;
    resumed
        .store
        .update(|s| {
            s.settings.notify_live = false;
            Ok(())
        })
        .await?;
    assert!(!resumed.snapshot().await?.settings.notify_live);
    resumed.shutdown().await?;
    Ok(())
}
#[tokio::test]
async fn removing_up_cleans_memberships_and_edit_preserves_unknown_fields() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let services = Services::open(dir.path().into()).await?;
    services.store.import_backup(json!({"ups":[{"mid":1,"name":"測試","future":42}],"settings":{"followGroups":[{"id":"group-a","name":"測試組","mids":[1]}]}})).await?;
    services
        .update_up(
            1,
            UpPatch {
                note: Some("新備註".into()),
                tags: Some(vec!["一".into(), "一".into()]),
                pinned: Some(true),
                ..Default::default()
            },
        )
        .await?;
    let snapshot = services.snapshot().await?;
    assert_eq!(snapshot.ups[0].extra["future"], 42);
    assert_eq!(snapshot.ups[0].tags.as_ref().unwrap().len(), 1);
    services.remove_up(1).await?;
    let snapshot = services.snapshot().await?;
    assert!(snapshot.ups.is_empty());
    assert!(snapshot.settings.follow_groups.unwrap()[0].mids.is_empty());
    Ok(())
}
