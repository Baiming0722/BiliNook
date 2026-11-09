use anyhow::Result;
use bili_core::{api::BiliApi, live_watch::*, models::*, store::JsonStore};
use futures::future::BoxFuture;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn room(id: u64, status: RoomStatus, at: u64) -> ObservedRoom {
    ObservedRoom {
        room_id: id,
        uid: id + 100,
        uname: format!("主播 {id}"),
        title: "測試直播".into(),
        status,
        last_fetched_at: iso(at),
        ..Default::default()
    }
}
fn document() -> LiveWatchData {
    let mut data = LiveWatchData::default();
    data.add_room(room(1, RoomStatus::Offline, 1000)).unwrap();
    data
}

#[test]
fn inputs_resolve_safe_ids_and_reject_foreign_hosts() {
    for text in [
        "12",
        " https://live.bilibili.com/12?x=1 ",
        "live.bilibili.com/blanc/12",
    ] {
        assert_eq!(parse_room_input(text).unwrap(), 12);
    }
    for text in [
        "",
        "0",
        "-1",
        "9007199254740992",
        "https://live.bilibili.com.evil/12",
        "https://evil/live.bilibili.com/12",
    ] {
        assert!(parse_room_input(text).is_err(), "{text}");
    }
}

#[test]
fn stale_sorting_cannot_drop_rooms_and_removed_groups_keep_members() -> Result<()> {
    let mut data = document();
    data.add_room(room(2, RoomStatus::Offline, 1000))?;
    let group = data.save_group(None, "測試分組".into())?;
    for id in [1, 2] {
        data.patch_room(
            id,
            RoomPatch {
                group_id: Some(Some(group.clone())),
                ..Default::default()
            },
        )?;
    }
    let before = data.clone();
    assert!(
        data.reorder_rooms(vec![1], Some(group.clone()), false)
            .is_err()
    );
    assert!(
        data.reorder_rooms(vec![1, 1], Some(group.clone()), false)
            .is_err()
    );
    assert_eq!(data, before);
    data.reorder_rooms(vec![2, 1], Some(group.clone()), false)?;
    assert_eq!(data.rooms.iter().find(|r| r.room_id == 2).unwrap().order, 0);
    data.remove_group(&group)?;
    assert_eq!(data.rooms.len(), 2);
    assert!(data.rooms.iter().all(|r| r.group_id.is_none()));
    data.validate()
}

#[test]
fn watchlist_updates_keep_identity_and_preserve_removed_rooms() -> Result<()> {
    let mut data = document();
    let id = data.save_watchlist(Watchlist {
        name: "同屏".into(),
        room_ids: vec![1, 99],
        playback_limit: 9,
        ..Default::default()
    })?;
    let created = data.watchlists[0].created_at.clone();
    let mut edited = data.watchlists[0].clone();
    edited.name = "新名稱".into();
    edited.room_ids.reverse();
    assert_eq!(data.save_watchlist(edited.clone())?, id);
    assert_eq!(data.watchlists.len(), 1);
    assert_eq!(data.watchlists[0].created_at, created);
    data.remove_rooms(&[1]);
    assert_eq!(data.watchlists[0].room_ids, vec![99, 1]);
    edited.playback_limit = 3;
    assert!(data.save_watchlist(edited).is_err());
    data.validate()
}
#[test]
fn status_precedence_and_unknown_values_are_preserved() {
    for (status, hidden, lock, want) in [
        (1, 0, 0, RoomStatus::Live),
        (2, 0, 0, RoomStatus::Round),
        (0, 0, 0, RoomStatus::Offline),
        (1, 1, 0, RoomStatus::Hidden),
        (1, 0, 1, RoomStatus::Locked),
        (99, 0, 0, RoomStatus::Unknown),
    ] {
        let info=normalize_info(1,&json!({"data":{"by_room_ids":{"1":{"room_id":1,"live_status":status,"hidden_status":hidden,"lock_status":lock}}}}),1).unwrap();
        assert_eq!(info.status, want);
    }
    assert_eq!(
        normalize_info(1, &json!({"data":{"by_room_ids":{"1":{"room_id":1}}}}), 1)
            .unwrap()
            .status,
        RoomStatus::Unknown
    );
}
#[test]
fn linked_and_manual_membership_survive_removal_and_undo() {
    let mut data = document();
    let up = UpUser {
        mid: 10,
        live_room: Some(LiveRoom {
            roomid: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    data.sync_ups(&[up.clone()]);
    assert_eq!(data.rooms.len(), 1);
    assert!(data.native.tracking[&1].manual);
    let undo = data.remove_rooms(&[1]);
    data.sync_ups(&[up.clone()]);
    assert!(data.rooms.is_empty());
    data.restore_rooms(undo).unwrap();
    data.sync_ups(&[]);
    assert_eq!(data.rooms.len(), 1);
    let mut auto = LiveWatchData::default();
    auto.sync_ups(&[up]);
    assert_eq!(auto.rooms.len(), 1);
    auto.sync_ups(&[]);
    assert!(auto.rooms.is_empty());
}
#[test]
fn legacy_import_deduplicates_and_rejects_invalid_history() {
    let data = LiveWatchData::from_legacy_value(
        json!({"version":1,"rooms":[{"roomId":1,"note":"先"},{"roomId":1,"note":"後"}]}),
    )
    .unwrap();
    assert_eq!(data.rooms.len(), 1);
    assert_eq!(data.rooms[0].note, "先");
    assert!(data.native.tracking[&1].manual);
    for value in [
        json!({"version":9,"rooms":[]}),
        json!({"rooms":[{"roomId":0}]}),
        json!({"rooms":[],"stats":{"history":{"1":[{"startedAt":"invalid"}]}}}),
        json!({"rooms":[],"stats":{"history":{"1":[{"startedAt":iso(2000),"endedAt":iso(1000)}]}}}),
    ] {
        assert!(LiveWatchData::from_legacy_value(value).is_err());
    }
}
#[test]
fn merge_preserves_existing_settings_groups_lists_and_best_history() {
    let mut data = document();
    let group = data.save_group(None, "原分組".into()).unwrap();
    let list = Watchlist {
        name: "常用".into(),
        room_ids: vec![1, 999],
        ..Default::default()
    };
    let list_id = data.save_watchlist(list).unwrap();
    data.stats.live_duration.insert(1, 100);
    data.stats.history.insert(
        1,
        vec![LiveSession {
            started_at: iso(1000),
            ended_at: Some(iso(5000)),
            incomplete: false,
        }],
    );
    data.import(json!({"version":2,"rooms":[{"roomId":2,"groupId":group}],"watchlists":[{"id":list_id,"name":"另一個","roomIds":[2]}],"settings":{"soundVolume":0.1},"stats":{"liveDuration":{"1":50},"history":{"1":[{"startedAt":iso(1000),"endedAt":iso(9000),"incomplete":true}]}}}),false).unwrap();
    assert_eq!(data.rooms.len(), 2);
    assert_eq!(data.watchlists[0].name, "常用");
    assert_eq!(data.settings.sound_volume, 0.5);
    assert_eq!(data.stats.live_duration[&1], 100);
    assert_eq!(data.stats.history[&1][0].ended_at, Some(iso(5000)));
    data.remove_rooms(&[1]);
    assert_eq!(data.watchlists[0].room_ids, vec![1, 999]);
    assert!(data.stats.history.contains_key(&1));
}
#[test]
fn observations_only_notify_real_transitions_and_never_bridge_errors() {
    let mut data = document();
    assert!(
        data.observe(vec![room(1, RoomStatus::Offline, 1000)], "a")
            .is_empty()
    );
    assert_eq!(
        data.observe(vec![room(1, RoomStatus::Live, 2000)], "a")
            .len(),
        1
    );
    assert!(
        data.observe(vec![room(1, RoomStatus::Live, 3000)], "a")
            .is_empty()
    );
    assert_eq!(data.stats.live_duration[&1], 1);
    data.observe(vec![room(1, RoomStatus::Error, 4000)], "a");
    assert!(
        data.observe(vec![room(1, RoomStatus::Live, 100_000)], "a")
            .is_empty()
    );
    assert_eq!(data.stats.live_duration[&1], 1);
    assert_eq!(data.native.notices.len(), 1);
    assert!(data.stats.history[&1][0].incomplete);
    assert_eq!(data.stats.history[&1][0].ended_at, Some(iso(3000)));
    data.observe(vec![room(1, RoomStatus::Offline, 101_000)], "a");
    data.observe(vec![room(1, RoomStatus::Error, 102_000)], "a");
    assert!(
        data.observe(vec![room(1, RoomStatus::Live, 103_000)], "a")
            .is_empty()
    );
}
#[test]
fn restart_closes_at_last_reliable_observation_and_ignores_stale_results() {
    let mut data = document();
    data.observe(vec![room(1, RoomStatus::Live, 1000)], "a");
    data.observe(vec![room(1, RoomStatus::Live, 2000)], "a");
    data.observe(vec![room(1, RoomStatus::Live, 1_000_000)], "b");
    assert_eq!(data.stats.history[&1][0].ended_at, Some(iso(2000)));
    assert!(data.stats.history[&1][0].incomplete);
    assert_eq!(data.stats.live_duration[&1], 1);
    data.observe(vec![room(1, RoomStatus::Offline, 1000)], "b");
    assert_eq!(data.native.cached_rooms[&1].status, RoomStatus::Live);
}
#[test]
fn history_csv_preserves_unknowns_intersections_and_formula_protection() {
    let mut data = document();
    data.native.cached_rooms.get_mut(&1).unwrap().uname = " =HYPERLINK(\"x\")".into();
    data.stats.history.insert(
        1,
        vec![
            LiveSession {
                started_at: iso(1000),
                ended_at: Some(iso(9000)),
                incomplete: false,
            },
            LiveSession {
                started_at: iso(10_000),
                ..Default::default()
            },
        ],
    );
    let rows = history_rows(&data, (4000, 20_000), None, 30_000);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].seconds, None);
    assert_eq!(rows[1].seconds, Some(5));
    assert!(rows[1].crosses_start);
    let csv = history_csv(&rows);
    assert!(csv.starts_with('\u{feff}'));
    assert!(csv.contains("' =HYPERLINK"));
    assert!(csv.contains("\r\n"));
    assert!(csv.contains("結束待確認"));
    assert!(local_date_range("2026-02-30", "2026-03-01").is_err());
    assert!(local_date_range("2026-03-02", "2026-03-01").is_err());
}
#[test]
fn quiet_hours_cross_midnight_and_snooze_expires() {
    let mut p = NotificationPreferences {
        quiet_enabled: true,
        ..Default::default()
    };
    assert!(reminders_paused(&p, 1, 23 * 60));
    assert!(reminders_paused(&p, 1, 7 * 60));
    assert!(!reminders_paused(&p, 1, 8 * 60));
    p.quiet_end = p.quiet_start.clone();
    assert!(!reminders_paused(&p, 1, 23 * 60));
    p.snooze_until = 100;
    assert!(reminders_paused(&p, 99, 12 * 60));
    assert!(!reminders_paused(&p, 100, 12 * 60));
}

#[test]
fn canonical_room_merge_keeps_stable_selection_and_best_history() {
    let mut data = document();
    data.add_room(room(99, RoomStatus::Live, 1000)).unwrap();
    data.rooms[1].note = "正式房號備註".into();
    let id = data
        .save_watchlist(Watchlist {
            name: "組合".into(),
            room_ids: vec![1, 99],
            ..Default::default()
        })
        .unwrap();
    data.stats.live_duration.insert(1, 20);
    data.stats.live_duration.insert(99, 10);
    data.canonicalize_room(1, 99);
    assert_eq!(data.rooms.len(), 1);
    assert_eq!(data.rooms[0].note, "正式房號備註");
    assert_eq!(
        data.watchlists
            .iter()
            .find(|w| w.id == id)
            .unwrap()
            .room_ids,
        vec![99]
    );
    assert_eq!(data.stats.live_duration[&99], 20);
    data.validate().unwrap();
}
#[test]
fn long_up_note_does_not_break_live_linking() {
    let mut data = LiveWatchData::default();
    data.sync_ups(&[UpUser {
        mid: 1,
        custom_note: Some("長".repeat(500)),
        live_room: Some(LiveRoom {
            roomid: 1,
            ..Default::default()
        }),
        ..Default::default()
    }]);
    assert_eq!(data.rooms[0].note.chars().count(), 200);
    data.validate().unwrap();
}
#[tokio::test]
async fn ascii_preference_accepts_old_settings_and_survives_integrated_backup() -> Result<()> {
    let old: AppSettings = serde_json::from_value(json!({"appearance":{"fontScale":1.2}}))?;
    assert!(old.appearance.ascii_animation_enabled);
    assert_eq!(old.appearance.font_scale, 1.2);
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update_both(|follow, live| {
            follow.settings.appearance.ascii_animation_enabled = false;
            *live = document();
            Ok(())
        })
        .await?;
    let backup = store.export_backup().await?;
    assert_eq!(
        backup["settings"]["appearance"]["asciiAnimationEnabled"],
        false
    );
    store
        .update(|s| {
            s.settings.appearance.ascii_animation_enabled = true;
            Ok(())
        })
        .await?;
    store.import_backup(backup).await?;
    assert!(
        !store
            .snapshot()
            .await?
            .settings
            .appearance
            .ascii_animation_enabled
    );
    assert_eq!(store.live_snapshot().await?.rooms[0].room_id, 1);
    Ok(())
}

#[tokio::test]
async fn live_and_follow_files_recover_together_before_corrupt_targets_are_read() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let live = document();
    let previous = json!({
        "ups": [{"mid":11,"name":"復原前追蹤"}],
        "settings": AppSettings::default(),
        "state": PollerState::default(),
        "readDynamics": ["read-before-import"],
        "liveWatch": live,
    });
    std::fs::write(
        dir.path().join(".import-rollback.json"),
        serde_json::to_vec(&previous)?,
    )?;
    std::fs::write(dir.path().join("ups.json"), "{interrupted")?;
    std::fs::write(dir.path().join("live-watch.json"), "{interrupted")?;
    let store = JsonStore::open(dir.path()).await?;
    let follow = store.snapshot().await?;
    assert_eq!(follow.ups[0].mid, 11);
    assert_eq!(follow.read_dynamics, vec!["read-before-import"]);
    assert_eq!(store.live_snapshot().await?.as_ref(), &live);
    assert!(!dir.path().join(".import-rollback.json").exists());
    Ok(())
}

#[tokio::test]
async fn actor_serializes_concurrent_live_edits_and_rejects_invalid_writes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update_live(|d| {
            *d = document();
            Ok(())
        })
        .await?;
    let (a, b) = tokio::join!(
        store.update_live(|d| d.patch_room(
            1,
            RoomPatch {
                note: Some("保留".into()),
                ..Default::default()
            }
        )),
        store.update_live(|d| d.patch_room(
            1,
            RoomPatch {
                pinned: Some(true),
                ..Default::default()
            }
        ))
    );
    a?;
    b?;
    let data = store.live_snapshot().await?;
    assert_eq!(data.rooms[0].note, "保留");
    assert!(data.rooms[0].pinned);
    let before = std::fs::read(dir.path().join("live-watch.json"))?;
    assert!(
        store
            .update_live(|d| {
                d.rooms[0].room_id = 0;
                Ok(())
            })
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(dir.path().join("live-watch.json"))?, before);
    Ok(())
}
#[tokio::test]
async fn integrated_backup_is_compatible_and_invalid_live_data_cannot_change_follow_data()
-> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update_live(|d| {
            *d = document();
            Ok(())
        })
        .await?;
    let backup = store.export_backup().await?;
    assert_eq!(backup["liveWatch"]["rooms"][0]["roomId"], 1);
    store
        .import_backup(json!({"version":"2.0.0","ups":[{"mid":12,"name":"原有"}]}))
        .await?;
    assert_eq!(store.live_snapshot().await?.rooms.len(), 1);
    assert!(
        store
            .import_backup(json!({"ups":[],"liveWatch":{"rooms":[{"roomId":0}]}}))
            .await
            .is_err()
    );
    assert_eq!(store.snapshot().await?.ups.len(), 1);
    Ok(())
}

#[test]
fn large_valid_live_backup_roundtrips() -> Result<()> {
    let mut data = LiveWatchData::default();
    for id in 1..=250 {
        data.rooms.push(StoredRoom {
            room_id: id,
            note: "備註".repeat(10),
            ..Default::default()
        });
        data.stats.history.insert(
            id,
            (0..50)
                .map(|minute| LiveSession {
                    started_at: format!("2026-01-01T00:{minute:02}:00.000Z"),
                    ended_at: Some(format!("2026-01-01T00:{minute:02}:30.000Z")),
                    incomplete: false,
                })
                .collect(),
        );
    }
    data.validate()?;
    let exported = serde_json::to_vec_pretty(&data.export_legacy())?;
    assert!(exported.len() > 1024 * 1024);
    let imported = LiveWatchData::from_legacy_value(parse_backup_bytes(&exported)?)?;
    assert_eq!(imported.rooms, data.rooms);
    assert_eq!(imported.stats.history.len(), data.stats.history.len());
    for (id, expected) in &data.stats.history {
        assert_eq!(imported.stats.history.get(id), Some(expected));
    }
    Ok(())
}

#[tokio::test]
async fn legacy_backup_restores_explicit_linked_notification_flags() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    store
        .update_both(|follow, live| {
            follow.ups.push(UpUser {
                mid: 1,
                name: "測試".into(),
                live_room: Some(LiveRoom {
                    roomid: 123,
                    ..Default::default()
                }),
                notify_on_live: Some(true),
                ..Default::default()
            });
            live.sync_ups(&follow.ups);
            Ok(())
        })
        .await?;
    store
        .import_backup(
            json!({"ups":[{"mid":1,"name":"測試","liveRoom":{"roomid":123},"notifyOnLive":false}]}),
        )
        .await?;
    assert_eq!(store.snapshot().await?.ups[0].notify_on_live, Some(false));
    assert!(!store.live_snapshot().await?.rooms[0].notify);
    store
        .import_backup(json!({"ups":[{"mid":1,"name":"測試","liveRoom":{"roomid":123}}]}))
        .await?;
    assert_eq!(store.snapshot().await?.ups[0].notify_on_live, Some(false));
    assert!(!store.live_snapshot().await?.rooms[0].notify);
    store
        .import_backup(
            json!({"ups":[{"mid":1,"name":"測試","liveRoom":{"roomid":123},"notifyOnLive":true}]}),
        )
        .await?;
    assert_eq!(store.snapshot().await?.ups[0].notify_on_live, Some(true));
    assert!(store.live_snapshot().await?.rooms[0].notify);
    Ok(())
}
struct MockApi {
    calls: AtomicUsize,
}
impl BiliApi for MockApi {
    fn room(&self, id: u64) -> BoxFuture<'_, Result<ObservedRoom>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(room(id, RoomStatus::Live, bili_core::now_ms()))
        })
    }
    fn live(&self, _uids: Vec<u64>) -> BoxFuture<'_, Result<BTreeMap<u64, LiveStatusInfo>>> {
        Box::pin(async { Ok(BTreeMap::new()) })
    }
    fn dynamics(&self, _mid: u64, _offset: String) -> BoxFuture<'_, Result<DynamicPage>> {
        Box::pin(async { Ok(DynamicPage::default()) })
    }
}
#[tokio::test]
async fn migration_reads_once_preserves_source_and_keeps_current_appearance() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("legacy.json");
    let text = serde_json::to_string(
        &json!({"version":2,"rooms":[{"roomId":1}],"settings":{"theme":"light","fontScale":1.5}}),
    )?;
    std::fs::write(&source, &text)?;
    let store = JsonStore::open(dir.path().join("new")).await?;
    let service = LiveWatchService::new(
        Arc::new(MockApi {
            calls: AtomicUsize::new(0),
        }),
        store.clone(),
    );
    service.migrate_once(Some(source.clone()), false).await?;
    assert_eq!(store.live_snapshot().await?.rooms.len(), 1);
    assert_eq!(store.snapshot().await?.settings.theme, "dark");
    store
        .update_live(|d| {
            d.rooms.clear();
            Ok(())
        })
        .await?;
    service.migrate_once(Some(source.clone()), false).await?;
    assert!(store.live_snapshot().await?.rooms.is_empty());
    assert_eq!(std::fs::read_to_string(source)?, text);
    Ok(())
}
#[tokio::test]
async fn corrupt_migration_keeps_error_and_can_be_retried() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let source = dir.path().join("bad.json");
    std::fs::write(&source, "broken")?;
    let store = JsonStore::open(dir.path().join("new")).await?;
    let service = LiveWatchService::new(
        Arc::new(MockApi {
            calls: AtomicUsize::new(0),
        }),
        store.clone(),
    );
    service.migrate_once(Some(source.clone()), false).await?;
    assert!(
        store
            .live_snapshot()
            .await?
            .native
            .migration_error
            .is_some()
    );
    std::fs::write(&source, "{\"rooms\":[{\"roomId\":1}]}")?;
    service.migrate_once(Some(source), true).await?;
    assert_eq!(store.live_snapshot().await?.rooms.len(), 1);
    assert!(
        store
            .live_snapshot()
            .await?
            .native
            .migration_error
            .is_none()
    );
    Ok(())
}
#[tokio::test]
async fn room_cache_coalesces_concurrent_queries_and_initial_live_does_not_notify() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = JsonStore::open(dir.path()).await?;
    let api = Arc::new(MockApi {
        calls: AtomicUsize::new(0),
    });
    let service = LiveWatchService::new(api.clone(), store);
    let (a, b) = tokio::join!(service.fetch_room(1, false), service.fetch_room(1, false));
    a?;
    b?;
    assert_eq!(api.calls.load(Ordering::Relaxed), 1);
    service.add_input("1".into()).await?;
    assert!(service.refresh(false).await?.is_empty());
    assert_eq!(api.calls.load(Ordering::Relaxed), 1);
    Ok(())
}
