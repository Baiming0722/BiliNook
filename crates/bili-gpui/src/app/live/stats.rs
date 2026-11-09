use super::*;
impl BiliApp {
    pub(super) fn live_number(
        &mut self,
        key: (u64, &'static str),
        value: u64,
        window: &mut Window,
        cx: &App,
    ) -> String {
        let motion = self
            .live
            .numbers
            .entry(key)
            .or_insert_with(|| MotionValue::settled(value as f32));
        motion.target(
            value as f32,
            motion::NORMAL,
            cx.reduce_motion() || self.live.keyboard,
        );
        if motion.active() {
            window.request_animation_frame();
        }
        ui::format_count(motion.value().round().max(0.) as u64)
    }
    pub(super) fn render_live_stats(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let data = self.live.snapshot.data.clone();
        let rooms = self.live.snapshot.rooms.clone();
        let live = rooms
            .iter()
            .filter(|r| r.info.status == RoomStatus::Live)
            .count();
        let offline = rooms
            .iter()
            .filter(|r| r.info.status == RoomStatus::Offline)
            .count();
        let errors = rooms
            .iter()
            .filter(|r| r.info.status == RoomStatus::Error)
            .count();
        let mut metrics = div().flex().flex_wrap().gap_3();
        for (index, (label, value)) in [
            ("監看房間", rooms.len()),
            ("直播中", live),
            ("未開播", offline),
            ("查詢錯誤", errors),
        ]
        .into_iter()
        .enumerate()
        {
            let text = self.live_number((index as u64, "summary"), value as u64, window, cx);
            metrics = metrics.child(
                div()
                    .flex_1()
                    .min_w(px(120.))
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                    .child(div().text_2xl().font_weight(FontWeight::BOLD).child(text)),
            );
        }
        let trend = seven_day_trend(&data, bili_core::now_ms());
        let max = trend.iter().map(|d| d.count).max().unwrap_or(1).max(1) as f32;
        let brand = cx.theme().primary;
        let muted = cx.theme().secondary;
        let mut chart = div()
            .h(px(210.))
            .flex()
            .items_end()
            .gap_3()
            .p_3()
            .rounded_lg()
            .bg(cx.theme().muted);
        for (index, day) in trend.into_iter().enumerate() {
            let motion = self
                .live
                .numbers
                .entry((index as u64, "trend"))
                .or_insert_with(|| MotionValue::settled(day.count as f32));
            motion.target(
                day.count as f32,
                motion::NORMAL,
                cx.reduce_motion() || self.live.keyboard,
            );
            if motion.active() {
                window.request_animation_frame();
            }
            let ratio = (motion.value() / max).clamp(0., 1.);
            chart = chart.child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(day.count.to_string()),
                    )
                    .child(
                        canvas(
                            |_, _, _| {},
                            move |bounds, _, window, _| {
                                window.paint_quad(fill(bounds, muted));
                                let height = bounds.size.height * ratio;
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: point(
                                            bounds.origin.x,
                                            bounds.origin.y + bounds.size.height - height,
                                        ),
                                        size: size(bounds.size.width, height),
                                    },
                                    brand,
                                ));
                            },
                        )
                        .h(px(120.))
                        .w(px(28.)),
                    )
                    .child(div().text_xs().child(day.label))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(panels::observed_duration(Some(day.duration_seconds))),
                    ),
            );
        }
        let mut content=div().p_4().flex().flex_col().gap_4().child(div().text_xs().text_color(cx.theme().muted_foreground).child("僅包含本工具監看期間的可靠觀測；每房保留最近 50 場。關閉與查詢失敗期間不推估時長。"))
            .child(metrics).child(div().font_weight(FontWeight::SEMIBOLD).child("近七日觀測場次與時長")).child(chart);
        for (index, (title, mut entries)) in [
            (
                "人氣排行",
                rooms
                    .iter()
                    .map(|r| (r.info.uname.clone(), r.info.online))
                    .collect::<Vec<_>>(),
            ),
            (
                "關注數排行",
                rooms
                    .iter()
                    .map(|r| (r.info.uname.clone(), r.info.attention))
                    .collect(),
            ),
            (
                "累計觀測時長",
                rooms
                    .iter()
                    .map(|r| {
                        (
                            r.info.uname.clone(),
                            data.stats
                                .live_duration
                                .get(&r.saved.room_id)
                                .copied()
                                .unwrap_or(0),
                        )
                    })
                    .collect(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            entries.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            content = content.child(div().font_weight(FontWeight::SEMIBOLD).child(title));
            for (rank, (name, value)) in entries.into_iter().take(5).enumerate() {
                content = content.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(cx.theme().muted)
                        .flex()
                        .justify_between()
                        .child(format!("{}. {name}", rank + 1))
                        .child(if index == 2 {
                            panels::observed_duration(Some(value))
                        } else {
                            ui::format_count(value)
                        }),
                );
            }
        }
        let mut areas = BTreeMap::<String, usize>::new();
        for room in &rooms {
            *areas
                .entry(if room.info.area_name.is_empty() {
                    "未分類".into()
                } else {
                    room.info.area_name.clone()
                })
                .or_default() += 1;
        }
        let mut areas: Vec<_> = areas.into_iter().collect();
        areas.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        content = content.child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("監看分區排行"),
        );
        for (area, count) in areas.into_iter().take(5) {
            content = content.child(div().text_sm().child(format!("{area}　{count} 間")));
        }
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let range = local_date_range(&today, &today).unwrap_or((0, 0));
        let rows = history_rows(&data, range, None, bili_core::now_ms());
        content = content.child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("今日觀測時間線"),
        );
        if rows.is_empty() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("今日尚無觀測場次。"),
            );
        }
        for row in rows.into_iter().take(10) {
            content = content.child(
                div()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(row.name)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {} · {}",
                                row.started_at,
                                row.state,
                                panels::observed_duration(row.seconds)
                            )),
                    ),
            );
        }
        crate::scroll::smooth_scroll_container(
            "live-stats-scroll",
            cx.entity_id(),
            &self.live.stats_scroll,
            content,
        )
        .into_any_element()
    }
}
