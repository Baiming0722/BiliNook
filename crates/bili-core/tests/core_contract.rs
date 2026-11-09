use anyhow::Result;
use bili_core::{
    api::{BiliApi, parse_comment, parse_dynamic, parse_profile},
    groups::*,
    models::*,
    poller::*,
    runtime::CoreRuntime,
    store::{JsonStore, atomic_write},
    wbi,
};
use futures::future::BoxFuture;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier},
};

fn up(mid: u64) -> UpUser {
    UpUser {
        mid,
        name: format!("測試 UP {mid}"),
        ..Default::default()
    }
}
fn snapshot() -> Snapshot {
    Snapshot {
        ups: vec![up(1), up(2)],
        ..Default::default()
    }
}
fn dynamic(id: &str, kind: DynamicType) -> DynamicItem {
    DynamicItem {
        id_str: id.into(),
        mid: 1,
        r#type: kind,
        ..Default::default()
    }
}

#[test]
fn old_settings_defaults_and_unknown_fields_roundtrip() -> Result<()> {
    let old = json!({"pollIntervalLiveSec":90,"theme":"system","future":{"nested":[1,2]}});
    let settings: AppSettings = serde_json::from_value(old.clone())?;
    assert_eq!(settings.poll_interval_dynamic_sec, 300);
    assert!(settings.enable_notifications && settings.sound_enabled);
    let saved = serde_json::to_value(settings)?;
    assert_eq!(saved["future"], old["future"]);
    assert_eq!(saved["pollIntervalLiveSec"], 90);
    Ok(())
}

#[test]
fn up_nested_unknown_fields_and_explicit_snake_case_are_preserved() -> Result<()> {
    let original = json!({"mid":1,"name":"測試","futureFlag":true,"liveRoom":{"roomid":42,"futureRoom":7},"official":{"role":0,"futureBadge":"x"}});
    let user: UpUser = serde_json::from_value(original)?;
    let saved = serde_json::to_value(user)?;
    assert_eq!(saved["futureFlag"], true);
    assert_eq!(saved["liveRoom"]["futureRoom"], 7);
    assert_eq!(saved["official"]["futureBadge"], "x");
    let item = DynamicItem {
        id_str: "123".into(),
        live_info: Some(DynamicLiveInfo {
            room_id: 42,
            ..Default::default()
        }),
        ..Default::default()
    };
    let saved = serde_json::to_value(item)?;
    assert_eq!(saved["id_str"], "123");
    assert_eq!(saved["liveInfo"]["room_id"], 42);
    assert!(saved.get("idStr").is_none());
    Ok(())
}

#[test]
fn wbi_matches_independent_dotnet_md5_vector() {
    let query = wbi::sign(
        &BTreeMap::from([
            ("foo".into(), "114".into()),
            ("bar".into(), "514".into()),
            ("baz".into(), "1919810".into()),
        ]),
        "7cd084941338484aae1ad9425b84077c",
        "4932caff0ff746eab6f01bf08b70ac45",
        1702204169,
    );
    assert_eq!(
        query,
        "bar=514&baz=1919810&foo=114&wts=1702204169&w_rid=6149fdadf571698ca7e6a567265cd0ee"
    );
}

#[test]
fn wbi_filters_illegal_characters_and_encodes_spaces_and_cjk() {
    let dirty = BTreeMap::from([("search".into(), "繁體 中文!'()*".into())]);
    let clean = BTreeMap::from([("search".into(), "繁體 中文".into())]);
    assert_eq!(
        wbi::sign(&dirty, "a", "b", 1),
        wbi::sign(&clean, "a", "b", 1)
    );
    assert!(wbi::sign(&dirty, "a", "b", 1).contains("%E7%B9%81%E9%AB%94%20%E4%B8%AD%E6%96%87"));
    assert!(!wbi::sign(&dirty, "a", "b", 1).contains('+'));
}

#[test]
fn groups_reject_reserved_duplicate_and_invalid_uid_and_deduplicate_members() {
    let mut groups = vec![FollowGroup {
        id: "all".into(),
        name: "測試".into(),
        ..Default::default()
    }];
    assert!(validate_groups(&mut groups).is_err());
    groups[0].id = "group-a".into();
    groups[0].mids = vec![1, 1, 2];
    validate_groups(&mut groups).unwrap();
    assert_eq!(groups[0].mids, vec![1, 2]);
    groups.push(FollowGroup {
        id: "group-b".into(),
        name: "測試".into(),
        ..Default::default()
    });
    assert!(validate_groups(&mut groups).is_err());
    groups.pop();
    groups[0].mids = vec![0];
    assert!(validate_groups(&mut groups).is_err());
}

#[test]
fn group_members_ignore_orphans_and_preserve_pin_order() {
    let mut users = vec![up(1), up(2), up(3)];
    users[2].is_pinned = Some(true);
    users[2].pin_order = Some(0);
    users[1].is_pinned = Some(true);
    users[1].pin_order = Some(1);
    sort_ups(&mut users);
    assert_eq!(
        users.iter().map(|u| u.mid).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    let groups = vec![FollowGroup {
        id: "group-a".into(),
        name: "組別".into(),
        mids: vec![2, 999],
        ..Default::default()
    }];
    assert_eq!(
        group_members(&users, &groups, "ungrouped")
            .iter()
            .map(|u| u.mid)
            .collect::<Vec<_>>(),
        vec![3, 1]
    );
    assert_eq!(group_members(&users, &groups, "group-a").len(), 1);
}

#[tokio::test]
async fn store_serializes_concurrent_updates_without_lost_ups() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    let mut tasks = Vec::new();
    for mid in 1..=64 {
        let store = store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .update(move |s| {
                    s.ups.push(up(mid));
                    Ok(())
                })
                .await
        }));
    }
    for task in tasks {
        task.await??;
    }
    store.flush().await?;
    let disk: Vec<UpUser> = serde_json::from_slice(&std::fs::read(dir.path().join("ups.json"))?)?;
    assert_eq!(disk.len(), 64);
    assert_eq!(store.snapshot().await?.ups.len(), 64);
    assert!(!dir.path().join(".import-rollback.json").exists());
    Ok(())
}

#[tokio::test]
async fn invalid_json_is_never_replaced_by_defaults() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("ups.json");
    std::fs::write(&path, "{broken")?;
    assert!(JsonStore::open(dir.path()).await.is_err());
    assert_eq!(std::fs::read_to_string(path)?, "{broken");
    Ok(())
}

#[tokio::test]
async fn same_data_directory_allows_only_one_writer() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    assert!(JsonStore::open(dir.path()).await.is_err());
    store.flush().await?;
    Ok(())
}

#[tokio::test]
async fn invalid_backup_does_not_partially_modify_data() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update(|s| {
            *s = snapshot();
            Ok(())
        })
        .await?;
    let before = store.snapshot().await?;
    assert!(store.import_backup(json!({"ups":[{"mid":5,"name":"valid"}],"settings":{"followGroups":[{"id":"all","name":"invalid","mids":[]}]}})).await.is_err());
    assert_eq!(store.snapshot().await?, before);
    assert!(
        store
            .import_backup(json!({"ups":[{"mid":1,"name":"a"},{"mid":1,"name":"b"}]}))
            .await
            .is_err()
    );
    assert_eq!(store.snapshot().await?, before);
    Ok(())
}

#[tokio::test]
async fn invalid_imported_settings_leave_existing_data_unchanged() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update(|s| {
            *s = snapshot();
            s.settings.theme = "light".into();
            Ok(())
        })
        .await?;
    let before = store.snapshot().await?;
    let before_disk = std::fs::read(dir.path().join("settings.json"))?;
    for settings in [
        json!({"customProxyUrl":"not-a-url"}),
        json!({"pollIntervalLiveSec":1}),
        json!({"pollIntervalDynamicSec":86401}),
        json!({"theme":"unsupported"}),
    ] {
        assert!(
            store
                .import_backup(json!({"ups":[], "settings":settings}))
                .await
                .is_err()
        );
        assert_eq!(store.snapshot().await?, before);
        assert_eq!(
            std::fs::read(dir.path().join("settings.json"))?,
            before_disk
        );
    }
    Ok(())
}

#[test]
fn oversized_atomic_write_preserves_original_file() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("data.json");
    atomic_write(&path, &json!({"previous":true}))?;
    let previous = std::fs::read(&path)?;
    let oversized = "x".repeat(32 * 1024 * 1024);
    let error = atomic_write(&path, &oversized).unwrap_err();
    assert!(error.to_string().contains("32 MiB"));
    assert_eq!(std::fs::read(path)?, previous);
    Ok(())
}

#[tokio::test]
async fn oversized_multi_file_update_rolls_back_earlier_writes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update(|s| {
            s.ups.push(up(1));
            s.settings.theme = "light".into();
            Ok(())
        })
        .await?;
    let before = store.snapshot().await?;
    let previous_ups = std::fs::read(dir.path().join("ups.json"))?;
    let previous_settings = std::fs::read(dir.path().join("settings.json"))?;
    let error = store
        .update(|s| {
            s.ups.push(up(2));
            s.settings.extra.insert(
                "largeField".into(),
                serde_json::Value::String("x".repeat(32 * 1024 * 1024)),
            );
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("32 MiB"));
    assert_eq!(store.snapshot().await?, before);
    assert_eq!(std::fs::read(dir.path().join("ups.json"))?, previous_ups);
    assert_eq!(
        std::fs::read(dir.path().join("settings.json"))?,
        previous_settings
    );
    assert!(!dir.path().join(".import-rollback.json").exists());
    Ok(())
}

#[tokio::test]
async fn legacy_backup_preserves_groups_and_roundtrips_read_dynamics() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update(|s| {
            s.settings.follow_groups = Some(default_groups());
            Ok(())
        })
        .await?;
    store.import_backup(json!({"ups":[{"mid":8,"name":"備份測試","customNote":"保留備註","future":7}],"settings":{"theme":"light"},"readDynamics":["1","1","2"]})).await?;
    let current = store.snapshot().await?;
    assert_eq!(current.settings.follow_groups.unwrap().len(), 2);
    assert_eq!(current.read_dynamics, vec!["1", "2"]);
    assert_eq!(current.ups[0].extra["future"], 7);
    let backup = store.export_backup().await?;
    assert_eq!(backup["version"], "3.0.0");
    assert_eq!(backup["liveWatch"]["version"], 2);
    assert_eq!(backup["readDynamics"], json!(["1", "2"]));
    assert!(backup.get("state").is_none());
    Ok(())
}

#[tokio::test]
async fn interrupted_legacy_import_is_recovered_before_reading_corrupt_target() -> Result<()> {
    let dir = tempfile::tempdir()?;
    atomic_write(
        &dir.path().join(".import-rollback.json"),
        &json!({"ups":[up(1)],"settings":{"theme":"light"}}),
    )?;
    std::fs::write(dir.path().join("ups.json"), "{interrupted")?;
    let store = JsonStore::open(dir.path()).await?;
    assert_eq!(store.snapshot().await?.ups[0].mid, 1);
    assert_eq!(store.snapshot().await?.settings.theme, "light");
    assert!(!dir.path().join(".import-rollback.json").exists());
    Ok(())
}

#[tokio::test]
async fn read_ids_keep_newest_5000_and_no_duplicates() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .mark_read((0..5000).map(|i| i.to_string()).collect())
        .await?;
    store.mark_read(vec!["4999".into(), "5000".into()]).await?;
    let ids = store.snapshot().await?.read_dynamics;
    assert_eq!(ids.len(), 5000);
    assert_eq!(ids[0], "1");
    assert_eq!(ids[4999], "5000");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_caller_does_not_cancel_an_accepted_write() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = barrier.clone();
    let (entered, waiting) = async_channel::bounded(1);
    let writer = store.clone();
    let task = tokio::spawn(async move {
        writer
            .update(move |s| {
                entered.try_send(()).unwrap();
                worker_barrier.wait();
                s.ups.push(up(9));
                Ok(())
            })
            .await
    });
    waiting.recv().await?;
    task.abort();
    barrier.wait();
    store.flush().await?;
    assert_eq!(store.snapshot().await?.ups[0].mid, 9);
    let disk: Vec<UpUser> = serde_json::from_slice(&std::fs::read(dir.path().join("ups.json"))?)?;
    assert_eq!(disk[0].mid, 9);
    Ok(())
}

#[tokio::test]
async fn failed_write_is_reported_and_memory_is_not_committed() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    std::fs::create_dir(dir.path().join("ups.json"))?;
    assert!(
        store
            .update(|s| {
                s.ups.push(up(7));
                Ok(())
            })
            .await
            .is_err()
    );
    assert!(store.snapshot().await?.ups.is_empty());
    assert!(store.flush().await.is_err());
    store.flush().await?;
    store
        .update(|s| {
            s.settings.theme = "light".into();
            Ok(())
        })
        .await?;
    assert_eq!(store.snapshot().await?.settings.theme, "light");
    Ok(())
}

#[test]
fn raw_video_preserves_comment_oid_and_timestamp_units() {
    let item=parse_dynamic(&json!({"id_str":"1234567890123456789","type":"DYNAMIC_TYPE_AV","basic":{"comment_id_str":"789","comment_type":"1"},
        "modules":{"module_author":{"mid":1,"name":"測試","face":"//i0.hdslb.com/face.png","pub_ts":1700000000},"module_dynamic":{"major":{"archive":{"aid":"789","bvid":"BV1234567890","title":"影片","cover":"//i0.hdslb.com/cover.jpg"}}}}}),0).unwrap();
    assert_eq!(item.r#type, DynamicType::Video);
    assert_eq!(item.pub_time, 1700000000000);
    assert_eq!(item.comment_id.as_deref(), Some("789"));
    assert_eq!(item.comment_type, Some(1));
    assert!(item.author_face.starts_with("https://"));
    assert_eq!(item.id_str, "1234567890123456789");
}

#[test]
fn draw_dynamic_with_opus_payload_keeps_text_and_images() {
    let raw = json!({
        "id_str": "42",
        "type": "DYNAMIC_TYPE_DRAW",
        "modules": {
            "module_dynamic": {
                "desc": null,
                "major": {
                    "type": "MAJOR_TYPE_OPUS",
                    "opus": {
                        "summary": { "text": "圖文內文\n第二行" },
                        "pics": [{ "url": "//i0.hdslb.com/example.jpg" }]
                    }
                }
            }
        }
    });
    let item = parse_dynamic(&raw, 1).unwrap();
    assert_eq!(item.r#type, DynamicType::Draw);
    assert_eq!(item.text.as_deref(), Some("圖文內文\n第二行"));
    assert_eq!(
        item.images.as_deref(),
        Some(["https://i0.hdslb.com/example.jpg".to_owned()].as_slice())
    );

    let mut rich_only = raw.clone();
    rich_only["modules"]["module_dynamic"]["major"]["opus"]["summary"] = json!({
        "rich_text_nodes": [{"orig_text": "第一段"}, {"text": "第二段"}]
    });
    assert_eq!(
        parse_dynamic(&rich_only, 1).unwrap().text.as_deref(),
        Some("第一段第二段")
    );

    let legacy = json!({
        "id_str": "43", "type": "DYNAMIC_TYPE_DRAW",
        "modules": {"module_dynamic": {
            "desc": {"text": "舊版圖文"},
            "major": {"draw": {"items": [{"src": "//i0.hdslb.com/legacy.jpg"}]}}
        }}
    });
    let item = parse_dynamic(&legacy, 1).unwrap();
    assert_eq!(item.text.as_deref(), Some("舊版圖文"));
    assert_eq!(item.images.unwrap(), ["https://i0.hdslb.com/legacy.jpg"]);
}

#[test]
fn reservation_card_is_optional_and_preserves_visible_details() {
    let mut raw = json!({
        "id_str": "42",
        "type": "DYNAMIC_TYPE_DRAW",
        "modules": {"module_dynamic": {
            "major": {"opus": {"summary": {"text": "內容"}, "pics": []}},
            "additional": {"type": "ADDITIONAL_TYPE_RESERVE", "reserve": {
                "title": "週年紀念回",
                "desc1": {"text": "11-15 18:30 直播"},
                "desc2": {"text": "181人預約", "visible": true}
            }}
        }}
    });
    let item = parse_dynamic(&raw, 1).unwrap();
    let reserve = item.reservation.unwrap();
    assert_eq!(reserve.title, "週年紀念回");
    assert_eq!(reserve.time.as_deref(), Some("11-15 18:30 直播"));
    assert_eq!(reserve.detail.as_deref(), Some("181人預約"));
    raw["modules"]["module_dynamic"]["additional"]["reserve"]["desc2"]["visible"] = json!(false);
    assert_eq!(
        parse_dynamic(&raw, 1).unwrap().reservation.unwrap().detail,
        None
    );

    let old: DynamicItem = serde_json::from_value(json!({"id_str": "old"})).unwrap();
    assert!(old.reservation.is_none());
    assert!(
        serde_json::to_value(old)
            .unwrap()
            .get("reservation")
            .is_none()
    );
}

#[test]
fn opus_draw_text_and_forward_parse_without_losing_original() {
    let mut raw = json!({"id_str":"10","type":"DYNAMIC_TYPE_OPUS","modules":{"module_dynamic":{"major":{"opus":{"summary":{"text":"圖文內容"},"pics":[{"url":"//i0.hdslb.com/1.png"},{"url":"//i0.hdslb.com/2.png"}]}}}}});
    let item = parse_dynamic(&raw, 1).unwrap();
    assert_eq!(item.r#type, DynamicType::Draw);
    assert_eq!(item.images.unwrap().len(), 2);
    raw["modules"]["module_dynamic"]["major"]["opus"]["pics"] = json!([]);
    assert_eq!(parse_dynamic(&raw, 1).unwrap().r#type, DynamicType::Text);
    let forward = parse_dynamic(
        &json!({"id_str":"11","type":"DYNAMIC_TYPE_FORWARD","orig":raw}),
        1,
    )
    .unwrap();
    assert_eq!(forward.r#type, DynamicType::Forward);
    assert_eq!(forward.orig.unwrap().text.as_deref(), Some("圖文內容"));
}

#[test]
fn encoded_live_card_and_comment_children_are_normalized() {
    let item=parse_dynamic(&json!({"id_str":"99","type":"DYNAMIC_TYPE_LIVE_RCMD","modules":{"module_dynamic":{"major":{"live_rcmd":{"content":"{\"live_play_info\":{\"room_id\":42,\"title\":\"直播測試\",\"cover\":\"//i0.hdslb.com/a.png\",\"live_status\":2}}"}}}}}),1).unwrap();
    assert_eq!(item.live_info.unwrap().live_status, Some(2));
    let comment = parse_comment(
        &json!({"rpid_str":"1234567890123456789","ctime":10,"member":{"uname":"訪客"},"content":{"message":"測試評論"},"replies":[{"rpid":2,"content":{"message":"回覆"}}]}),
    );
    assert_eq!(comment.pub_time, 10000);
    assert_eq!(comment.rpid, "1234567890123456789");
    assert_eq!(comment.sub_replies.unwrap()[0].message, "回覆");
}

#[test]
fn card_profile_normalizes_numeric_strings() -> Result<()> {
    let profile = parse_profile(
        &json!({"data":{"card":{"mid":"123","name":"測試","level_info":{"current_level":6}},"live":{"roomid":"42","liveStatus":1}}}),
        true,
    )?;
    assert_eq!(profile.mid, 123);
    assert_eq!(profile.level, 6);
    assert_eq!(profile.live_room.unwrap().roomid, 42);
    Ok(())
}

#[test]
fn initial_dynamics_seed_silently_then_notify_once() {
    let mut state = snapshot();
    let page = DynamicPage {
        items: vec![dynamic("old", DynamicType::Text)],
        ..Default::default()
    };
    let mut pages = BTreeMap::from([(1, page)]);
    assert!(reconcile_dynamics(&mut state, &pages, 1).is_empty());
    pages
        .get_mut(&1)
        .unwrap()
        .items
        .insert(0, dynamic("new", DynamicType::Text));
    assert_eq!(reconcile_dynamics(&mut state, &pages, 2).len(), 1);
    assert!(reconcile_dynamics(&mut state, &pages, 3).is_empty());
}

#[test]
fn notification_switches_do_not_stop_dedup_state_updates() {
    let mut state = snapshot();
    state
        .state
        .known_dynamic_ids
        .insert("1".into(), vec!["old".into()]);
    state.settings.enable_notifications = false;
    let pages = BTreeMap::from([(
        1,
        DynamicPage {
            items: vec![dynamic("new", DynamicType::Text)],
            ..Default::default()
        },
    )]);
    assert!(reconcile_dynamics(&mut state, &pages, 1).is_empty());
    state.settings.enable_notifications = true;
    assert!(reconcile_dynamics(&mut state, &pages, 2).is_empty());
    state.ups[0].notify_on_dynamic = Some(false);
    let pages = BTreeMap::from([(
        1,
        DynamicPage {
            items: vec![dynamic("newer", DynamicType::Text)],
            ..Default::default()
        },
    )]);
    assert!(reconcile_dynamics(&mut state, &pages, 3).is_empty());
}

#[test]
fn live_missing_uid_is_not_an_offline_transition_and_repeats_do_not_notify() {
    let mut state = snapshot();
    state.state.known_live_uids = vec![2];
    let statuses = BTreeMap::from([(
        1,
        LiveStatusInfo {
            uid: 1,
            live_status: 1,
            room_id: 42,
            ..Default::default()
        },
    )]);
    assert_eq!(reconcile_live(&mut state, &statuses, 1).len(), 1);
    assert_eq!(state.state.known_live_uids, vec![1, 2]);
    assert!(reconcile_live(&mut state, &statuses, 2).is_empty());
    state.ups[0].notify_on_live = Some(false);
    state.state.known_live_uids.clear();
    assert!(reconcile_live(&mut state, &statuses, 3).is_empty());
}

#[test]
fn fallback_pages_do_not_seed_notifications_and_history_is_bounded() {
    let mut state = snapshot();
    let pages = BTreeMap::from([(
        1,
        DynamicPage {
            items: vec![dynamic("fake", DynamicType::Video)],
            warning: Some("備援".into()),
            ..Default::default()
        },
    )]);
    assert!(reconcile_dynamics(&mut state, &pages, 1).is_empty());
    assert!(state.state.known_dynamic_ids.is_empty());
    let pages = BTreeMap::from([(
        1,
        DynamicPage {
            items: (0..100)
                .map(|id| dynamic(&id.to_string(), DynamicType::Text))
                .collect(),
            ..Default::default()
        },
    )]);
    reconcile_dynamics(&mut state, &pages, 2);
    assert_eq!(state.state.known_dynamic_ids["1"].len(), 50);
}

struct FailingApi;
impl BiliApi for FailingApi {
    fn live(&self, _: Vec<u64>) -> BoxFuture<'_, Result<BTreeMap<u64, LiveStatusInfo>>> {
        Box::pin(async { anyhow::bail!("樣本網路錯誤") })
    }
    fn dynamics(&self, _: u64, _: String) -> BoxFuture<'_, Result<DynamicPage>> {
        Box::pin(async { anyhow::bail!("樣本網路錯誤") })
    }
}

#[tokio::test]
async fn network_failures_preserve_known_live_and_dynamic_state() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update(|s| {
            *s = snapshot();
            s.state.known_live_uids = vec![1];
            s.state
                .known_dynamic_ids
                .insert("1".into(), vec!["old".into()]);
            Ok(())
        })
        .await?;
    let poller = Poller::new(Arc::new(FailingApi), store.clone());
    assert!(poller.poll_live().await.is_err());
    assert!(poller.poll_dynamic().await.is_err());
    let state = store.snapshot().await?.state;
    assert_eq!(state.known_live_uids, vec![1]);
    assert_eq!(state.known_dynamic_ids["1"], vec!["old"]);
    Ok(())
}

#[test]
fn core_runtime_provides_tokio_reactor_to_jobs_submitted_without_a_runtime() -> Result<()> {
    assert!(tokio::runtime::Handle::try_current().is_err());
    let runtime = CoreRuntime::start()?;
    let result = runtime
        .submit(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            Ok(listener.local_addr()?.port())
        })?
        .recv_blocking()??;
    assert!(result > 0);
    futures::executor::block_on(runtime.shutdown());
    assert!(runtime.submit(async { Ok(1) }).is_err());
    Ok(())
}

#[test]
fn shutdown_cancels_pending_io_and_drains_an_accepted_store_write() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let runtime = CoreRuntime::start()?;
    let path = dir.path().to_owned();
    let store = runtime
        .submit(async move { JsonStore::open(path).await })?
        .recv_blocking()??;
    let cancel = tokio_util::sync::CancellationToken::new();
    let pending =
        runtime.submit_cancellable(cancel.clone(), std::future::pending::<Result<()>>())?;
    let (accepted_tx, accepted_rx) = std::sync::mpsc::sync_channel(1);
    let write_store = store.clone();
    let writing = runtime.submit_cancellable(cancel.clone(), async move {
        write_store
            .update(move |snapshot| {
                snapshot.ups.push(up(77));
                let _ = accepted_tx.send(());
                Ok(())
            })
            .await
    })?;
    accepted_rx.recv_timeout(std::time::Duration::from_secs(5))?;
    cancel.cancel();
    assert!(pending.recv_blocking()?.is_err());
    let _ = writing.recv_blocking()?;
    runtime
        .submit(async move { store.flush().await })?
        .recv_blocking()??;
    futures::executor::block_on(runtime.shutdown());
    let ups: Vec<UpUser> = serde_json::from_slice(&std::fs::read(dir.path().join("ups.json"))?)?;
    assert_eq!(ups[0].mid, 77);
    Ok(())
}
