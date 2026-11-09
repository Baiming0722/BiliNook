use super::normalize_url;
use crate::models::*;
use anyhow::{Result, ensure};
use serde_json::Value;

pub(crate) fn number(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .unwrap_or_default()
}
pub(crate) fn string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}
pub(crate) fn url_string(value: &Value) -> String {
    normalize_url(&string(value))
}
fn optional(value: &Value) -> Option<String> {
    let value = string(value);
    (!value.is_empty()).then_some(value)
}

pub fn parse_profile(value: &Value, card: bool) -> Result<UpUser> {
    let data = &value["data"];
    let profile = if card { &data["card"] } else { data };
    let mut up = UpUser {
        mid: number(&profile["mid"]),
        name: string(&profile["name"]),
        face: url_string(&profile["face"]),
        sign: string(&profile["sign"]),
        sex: string(&profile["sex"]),
        level: number(if card {
            &profile["level_info"]["current_level"]
        } else {
            &profile["level"]
        }) as u32,
        ..Default::default()
    };
    ensure!(
        up.mid > 0 && !up.name.trim().is_empty(),
        "UP 主資料缺少有效 UID 或名稱"
    );
    let official = if card {
        &profile["Official"]
    } else {
        &profile["official"]
    };
    if official.is_object() {
        up.official = Some(serde_json::from_value(official.clone())?);
    }
    let live = if card {
        &data["live"]
    } else {
        &profile["live_room"]
    };
    if live.is_object() {
        up.live_room = Some(LiveRoom {
            room_status: 1,
            live_status: number(&live["liveStatus"]) as u32,
            url: url_string(&live["url"]),
            title: string(&live["title"]),
            cover: url_string(&live["cover"]),
            online: number(&live["online"]),
            roomid: number(&live["roomid"]),
            ..Default::default()
        });
    }
    Ok(up)
}

pub(crate) fn parse_video(v: &Value, mid: u64, media: bool) -> VideoItem {
    let time = number(if media {
        &v["pubtime"]
    } else {
        v.get("created").unwrap_or(&v["pubdate"])
    });
    VideoItem {
        bvid: string(if media { &v["bv_id"] } else { &v["bvid"] }),
        aid: number(if media { &v["id"] } else { &v["aid"] }),
        title: string(&v["title"]),
        description: string(if media {
            &v["intro"]
        } else {
            &v["description"]
        }),
        pic: url_string(if media { &v["cover"] } else { &v["pic"] }),
        duration: if media {
            format_duration(number(&v["duration"]))
        } else {
            string(&v["length"])
        },
        pubdate: time,
        created: time,
        play: number(if media {
            &v["cnt_info"]["play"]
        } else {
            &v["play"]
        }),
        video_review: number(if media {
            &v["cnt_info"]["danmaku"]
        } else {
            &v["video_review"]
        }),
        comment: number(if media {
            &v["cnt_info"]["reply"]
        } else {
            &v["comment"]
        }),
        author: string(if media {
            &v["upper"]["name"]
        } else {
            &v["author"]
        }),
        author_face: v["upper"]["face"].as_str().map(normalize_url),
        mid: {
            let parsed = number(if media { &v["upper"]["mid"] } else { &v["mid"] });
            if parsed > 0 { parsed } else { mid }
        },
        ..Default::default()
    }
}

fn format_duration(seconds: u64) -> String {
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

pub fn parse_dynamic(value: &Value, fallback_mid: u64) -> Option<DynamicItem> {
    dynamic(value, fallback_mid, 0)
}

fn dynamic(value: &Value, fallback_mid: u64, depth: u32) -> Option<DynamicItem> {
    let id = string(&value["id_str"]);
    if id.is_empty() || depth > 4 {
        return None;
    }
    let author = &value["modules"]["module_author"];
    let module = &value["modules"]["module_dynamic"];
    let major = &module["major"];
    let raw = string(&value["type"]);
    let mut item = DynamicItem {
        id_str: id.clone(),
        raw_type: Some(raw.clone()),
        mid: {
            let mid = number(&author["mid"]);
            if mid > 0 { mid } else { fallback_mid }
        },
        author_name: string(&author["name"]),
        author_face: url_string(&author["face"]),
        pub_time: number(&author["pub_ts"]).saturating_mul(1000),
        pub_action: optional(&author["pub_action"]),
        text: optional(&module["desc"]["text"]),
        jump_url: Some(format!("https://t.bilibili.com/{id}")),
        ..Default::default()
    };
    if item.author_face.is_empty() {
        for path in [
            "/avatar",
            "/avatar/container_size/layers/0/general_spec/web_css_resource/image",
            "/avatar/fallback_layers/layers/0/general_spec/web_css_resource/image",
            "/avatar_url",
        ] {
            if let Some(face) = author.pointer(path).and_then(Value::as_str) {
                item.author_face = normalize_url(face);
                break;
            }
        }
    }
    // 外層 DYNAMIC_TYPE_DRAW 也可能搭配 MAJOR_TYPE_OPUS，先依實際內容判斷。
    let source = if major["archive"].is_object() {
        item.r#type = DynamicType::Video;
        item.bvid = optional(&major["archive"]["bvid"]);
        if item.text.is_none() {
            item.text = optional(&major["archive"]["desc"]);
        }
        &major["archive"]
    } else if major["article"].is_object() {
        item.r#type = DynamicType::Article;
        item.cover = optional(&major["article"]["covers"][0]).map(|s| normalize_url(&s));
        &major["article"]
    } else if major["opus"].is_object() {
        let pics: Vec<_> = major["opus"]["pics"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| optional(&v["url"]).map(|s| normalize_url(&s)))
            .collect();
        item.r#type = if !pics.is_empty() || raw == "DYNAMIC_TYPE_DRAW" {
            DynamicType::Draw
        } else {
            DynamicType::Text
        };
        if !pics.is_empty() {
            item.images = Some(pics);
        }
        if item.text.is_none() {
            let summary = &major["opus"]["summary"];
            item.text = optional(&summary["text"]).or_else(|| {
                let text = summary["rich_text_nodes"]
                    .as_array()?
                    .iter()
                    .filter_map(|node| {
                        optional(&node["orig_text"]).or_else(|| optional(&node["text"]))
                    })
                    .collect::<String>();
                (!text.is_empty()).then_some(text)
            });
        }
        &major["opus"]
    } else if major["draw"].is_object() {
        item.r#type = DynamicType::Draw;
        item.images = Some(
            major["draw"]["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| optional(&v["src"]).map(|s| normalize_url(&s)))
                .collect(),
        );
        &major["draw"]
    } else {
        item.r#type = match raw.as_str() {
            "DYNAMIC_TYPE_AV" => DynamicType::Video,
            "DYNAMIC_TYPE_ARTICLE" => DynamicType::Article,
            "DYNAMIC_TYPE_DRAW" => DynamicType::Draw,
            "DYNAMIC_TYPE_WORD" | "DYNAMIC_TYPE_OPUS" => DynamicType::Text,
            "DYNAMIC_TYPE_FORWARD" => DynamicType::Forward,
            _ => DynamicType::Other,
        };
        &Value::Null
    };
    let reserve = &module["additional"]["reserve"];
    if reserve.is_object() {
        let title = string(&reserve["title"]);
        let time = optional(&reserve["desc1"]["text"]);
        let detail = if reserve["desc2"]["visible"].as_bool() == Some(false) {
            None
        } else {
            optional(&reserve["desc2"]["text"])
        };
        if !title.is_empty() || time.is_some() || detail.is_some() {
            item.reservation = Some(DynamicReservation {
                title,
                time,
                detail,
            });
        }
    }
    item.title = optional(&source["title"]);
    if let Some(cover) = optional(&source["cover"]) {
        item.cover = Some(normalize_url(&cover));
    }
    if item.cover.is_none() {
        item.cover = item
            .images
            .as_ref()
            .and_then(|images| images.first().cloned());
    }
    if let Some(url) = optional(&source["jump_url"]) {
        item.jump_url = Some(normalize_url(&url));
    } else if let Some(bvid) = &item.bvid {
        item.jump_url = Some(format!("https://www.bilibili.com/video/{bvid}"));
    }
    if raw == "DYNAMIC_TYPE_LIVE_RCMD"
        || raw == "DYNAMIC_TYPE_LIVE"
        || major["live_rcmd"].is_object()
        || major["live"].is_object()
    {
        item.r#type = DynamicType::Live;
        let rcmd = &major["live_rcmd"];
        let content = if let Some(content) = rcmd["content"].as_str() {
            serde_json::from_str::<Value>(content).ok()
        } else {
            Some(rcmd["content"].clone())
        };
        let data = if rcmd["live_play_info"].is_object() {
            &rcmd["live_play_info"]
        } else if let Some(content) = &content {
            if content["live_play_info"].is_object() {
                &content["live_play_info"]
            } else if content.is_object() {
                content
            } else {
                &major["live"]
            }
        } else {
            &major["live"]
        };
        let room_id = number(data.get("room_id").unwrap_or(&data["id"]));
        let link = optional(data.get("link").unwrap_or(&data["jump_url"]))
            .map(|s| normalize_url(&s))
            .or_else(|| (room_id > 0).then(|| format!("https://live.bilibili.com/{room_id}")));
        let live = DynamicLiveInfo {
            room_id,
            title: string(&data["title"]),
            cover: url_string(&data["cover"]),
            area_name: string(data.get("area_name").unwrap_or(&data["badge"]["text"])),
            watched_show: optional(&data["watched_show"]["text_large"])
                .or_else(|| optional(&data["watched_show"]["num"])),
            live_status: Some(number(
                data.get("live_status")
                    .or_else(|| data.get("live_state"))
                    .unwrap_or(&Value::from(1)),
            ) as u32),
            link: link.clone(),
            ..Default::default()
        };
        item.title = Some(live.title.clone());
        item.cover = Some(live.cover.clone());
        if let Some(link) = link {
            item.jump_url = Some(link);
        }
        item.live_info = Some(live);
    }
    if let Some(stat) = value["modules"].get("module_stat") {
        item.stat = Some(DynamicStat {
            reply: number(&stat["comment"]["count"]),
            like: number(&stat["like"]["count"]),
            forward: number(&stat["forward"]["count"]),
        });
    }
    item.comment_id = optional(&value["basic"]["comment_id_str"]);
    item.comment_type = value["basic"]
        .get("comment_type")
        .map(|value| number(value) as u32)
        .filter(|kind| *kind > 0);
    if item.comment_id.is_none() {
        if item.r#type == DynamicType::Video && number(&major["archive"]["aid"]) > 0 {
            item.comment_id = Some(number(&major["archive"]["aid"]).to_string());
            item.comment_type = Some(1);
        } else {
            item.comment_id = Some(id);
        }
    }
    if item.comment_type.is_none() {
        item.comment_type = Some(if item.r#type == DynamicType::Video {
            1
        } else {
            11
        });
    }
    item.orig = value
        .get("orig")
        .and_then(|orig| dynamic(orig, 0, depth + 1))
        .map(Box::new);
    Some(item)
}

pub fn parse_comment(value: &Value) -> CommentItem {
    comment(value, true)
}
fn comment(value: &Value, include_children: bool) -> CommentItem {
    let children: Vec<_> = if include_children {
        value["replies"]
            .as_array()
            .into_iter()
            .flatten()
            .take(20)
            .map(|v| comment(v, false))
            .collect()
    } else {
        Vec::new()
    };
    let emotes = value["content"]["emote"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(token, item)| {
            let url = optional(&item["gif_url"])
                .or_else(|| optional(&item["url"]))
                .map(|raw| normalize_url(&raw))
                .filter(|u| !u.is_empty())?;
            let size = (number(&item["meta"]["size"]) as u32).clamp(1, 2);
            Some((token.clone(), CommentEmote { url, size }))
        })
        .collect();
    let pictures = value["content"]["pictures"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pic| {
            optional(&pic["img_src"])
                .map(|raw| normalize_url(&raw))
                .filter(|u| !u.is_empty())
        })
        .collect();
    CommentItem {
        rpid: string(value.get("rpid_str").unwrap_or(&value["rpid"])),
        mid: number(&value["mid"]),
        uname: string(&value["member"]["uname"]),
        avatar: url_string(&value["member"]["avatar"]),
        level: number(&value["member"]["level_info"]["current_level"]) as u32,
        pub_time: number(&value["ctime"]).saturating_mul(1000),
        like: number(&value["like"]),
        message: string(&value["content"]["message"]),
        emotes,
        pictures,
        sub_reply_count: Some(number(&value["rcount"]).max(children.len() as u64)),
        sub_replies: (!children.is_empty()).then_some(children),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_parser_extracts_emotes_and_pictures_recursively() {
        let raw = serde_json::json!({
            "rpid_str": "9001",
            "mid": 42,
            "ctime": 1700000000,
            "like": 18,
            "rcount": 1,
            "member": {
                "uname": "評論者",
                "avatar": "//i0.hdslb.com/bfs/face/member.jpg",
                "level_info": { "current_level": 6 }
            },
            "content": {
                "message": "太強了[doge][UP主_打call]",
                "emote": {
                    "[doge]": {
                        "url": "//i0.hdslb.com/bfs/emote/doge.png",
                        "meta": { "size": 1 }
                    },
                    "[UP主_打call]": {
                        "url": "https://i0.hdslb.com/bfs/emote/call.png",
                        "gif_url": "//i0.hdslb.com/bfs/emote/call.gif",
                        "meta": { "size": 2 }
                    }
                },
                "pictures": [
                    { "img_src": "//i0.hdslb.com/bfs/new_dyn/pic1.webp" }
                ]
            },
            "replies": [
                {
                    "rpid_str": "9002",
                    "mid": 43,
                    "ctime": 1700000010,
                    "like": 2,
                    "member": {
                        "uname": "樓中樓",
                        "avatar": "https://i0.hdslb.com/bfs/face/sub.jpg",
                        "level_info": { "current_level": 5 }
                    },
                    "content": {
                        "message": "附議[妙啊]",
                        "emote": {
                            "[妙啊]": {
                                "url": "//i0.hdslb.com/bfs/emote/miao.png",
                                "meta": { "size": 1 }
                            }
                        }
                    }
                }
            ]
        });

        let parsed = parse_comment(&raw);
        assert_eq!(parsed.emotes.len(), 2);
        assert_eq!(
            parsed.emotes["[doge]"].url,
            "https://i0.hdslb.com/bfs/emote/doge.png"
        );
        assert_eq!(parsed.emotes["[doge]"].size, 1);
        assert_eq!(
            parsed.emotes["[UP主_打call]"].url,
            "https://i0.hdslb.com/bfs/emote/call.gif"
        );
        assert_eq!(parsed.emotes["[UP主_打call]"].size, 2);
        assert_eq!(
            parsed.pictures,
            vec!["https://i0.hdslb.com/bfs/new_dyn/pic1.webp"]
        );
        let sub = &parsed.sub_replies.expect("sub replies")[0];
        assert_eq!(
            sub.emotes["[妙啊]"].url,
            "https://i0.hdslb.com/bfs/emote/miao.png"
        );
    }
}
