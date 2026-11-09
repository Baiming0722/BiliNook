use super::*;
use crate::{
    desktop::DesktopEvent,
    ui::{self, Tone},
};
use bili_core::poller::Notice;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct QuitRecoveryState {
    pub quitting: bool,
    pub io_accepting: bool,
    pub show_overlay: bool,
}

pub(super) fn transition_after_shutdown_failure(
    token: &mut tokio_util::sync::CancellationToken,
) -> QuitRecoveryState {
    // 退出排空失敗時必須替換已被 cancel 的 CancellationToken，否則後續 `BiliApp::run`
    // 會一律早退，使應用在退出覆蓋層與主畫面皆無法再執行任何 I/O。
    *token = tokio_util::sync::CancellationToken::new();
    QuitRecoveryState {
        quitting: false,
        io_accepting: !token.is_cancelled(),
        show_overlay: true,
    }
}

impl BiliApp {
    pub(super) fn install_desktop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| {
                if !this.quitting && this.shutdown_error.is_none() && this.desktop.has_tray() {
                    this.desktop.hide();
                } else {
                    this.request_quit(cx);
                }
                false
            })
            .unwrap_or(true)
        });
        let weak = cx.entity().downgrade();
        window
            .observe_window_appearance(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if this.active_theme_mode() == "system" {
                        this.apply_theme(cx);
                        cx.notify();
                    }
                });
            })
            .detach();
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
                if weak
                    .update(cx, |this, cx| {
                        this.poll_live_players(cx);
                        let foreground = this.desktop.is_foreground();
                        let ascii_enabled =
                            this.snapshot.settings.appearance.ascii_animation_enabled;
                        this.ascii.update(cx, |clock, cx| {
                            clock.configure(ascii_enabled, cx);
                            clock.tick(foreground, cx);
                        });
                        for event in this.desktop.poll() {
                            match event {
                                DesktopEvent::Show => this.desktop.show(),
                                DesktopEvent::Toggle => this.desktop.toggle(),
                                DesktopEvent::Quit => this.request_quit(cx),
                            }
                        }
                        let responses = this.notifications.responses().collect::<Vec<_>>();
                        for response in responses {
                            match response {
                                crate::notifications::Response::Open(target) => {
                                    this.desktop.show();
                                    if let Some(target) = target {
                                        match target {
                                            crate::notifications::Target::External(url) => {
                                                this.open_url(url, cx)
                                            }
                                            crate::notifications::Target::LiveRoom(id) => {
                                                if !this.live.panels.is_empty() {
                                                    this.live.pending_focus = Some(id);
                                                } else {
                                                    let handle = this.window;
                                                    let weak = cx.entity().downgrade();
                                                    cx.defer(move |cx| {
                                                        let _ =
                                                            handle.update(cx, |_, window, cx| {
                                                                let _ =
                                                                    weak.update(cx, |this, cx| {
                                                                        this.focus_live_room(
                                                                            id, window, cx,
                                                                        )
                                                                    });
                                                            });
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                                crate::notifications::Response::Failed(error) => {
                                    this.error = Some(error);
                                    cx.notify();
                                }
                            }
                        }
                        let live = this
                            .snapshot
                            .ups
                            .iter()
                            .filter(|up| up.live_room.as_ref().is_some_and(|r| r.live_status == 1))
                            .count();
                        let live = if this.live.snapshot.rooms.is_empty() {
                            live
                        } else {
                            this.live
                                .snapshot
                                .rooms
                                .iter()
                                .filter(|r| {
                                    r.info.status == bili_core::live_watch::RoomStatus::Live
                                })
                                .count()
                        };
                        this.desktop
                            .update_tooltip(live, this.live_busy || this.dynamic_busy);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(super) fn native_notice(&mut self, notice: &Notice, cx: &mut Context<Self>) {
        let live_id = notice
            .url
            .strip_prefix("bili-live:")
            .and_then(|value| value.parse::<u64>().ok());
        if live_id.is_some()
            && self.snapshot.settings.sound_enabled
            && self.live.snapshot.data.settings.sound_enabled
        {
            crate::audio::play(true, self.live.snapshot.data.settings.sound_volume);
        }
        if live_id.is_some() && !self.live.snapshot.data.settings.notify_on_live {
            return;
        }
        let tag = format!("bili-{:?}-{}", notice.kind, notice.mid);
        if let Err(error) = self.notifications.show(
            tag,
            &notice.title,
            &notice.body,
            Some(
                live_id
                    .map(crate::notifications::Target::LiveRoom)
                    .unwrap_or_else(|| crate::notifications::Target::External(notice.url.clone())),
            ),
            live_id.is_none() && self.snapshot.settings.sound_enabled,
        ) {
            self.error = Some(format!("系統通知：{error}"));
            cx.notify();
        }
    }

    pub(super) fn test_notification(&mut self, cx: &mut Context<Self>) {
        match self.notifications.show(
            "bili-test".into(),
            bili_core::APP_NAME,
            "原生通知測試。關閉主視窗後，應用仍會在系統托盤持續追蹤。",
            None,
            self.settings.general_sound_enabled(),
        ) {
            Ok(()) => {
                self.show_notice("已送交 Windows 通知中心；顯示方式依系統通知設定".into(), cx)
            }
            Err(error) => {
                self.error = Some(format!("系統通知：{error}"));
                cx.notify();
            }
        }
    }

    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        self.quitting = true;
        self.live.stop();
        self.confirm_force_quit = false;
        self.shutdown_error = None;
        self.io_stop.cancel();
        let services = self.services.clone();
        let runtime = self.runtime.clone();
        let player_host = self.live.host.clone();
        // 這個工作不使用可取消橋接；必須先停止輪詢並收到存檔 actor 的 flush 確認。
        let result = self.runtime.submit(async move {
            if let Some(services) = services {
                services.shutdown().await?;
            }
            Ok(())
        });
        match result {
            Ok(result) => {
                cx.spawn(async move |weak, cx| match result.recv().await {
                    Ok(Ok(())) => {
                        runtime.shutdown().await;
                        // Wry's child-window handle must remain valid until its
                        // in-flight WebView2 construction has returned.
                        while player_host.borrow().creating() {
                            cx.background_executor()
                                .timer(Duration::from_millis(30))
                                .await;
                        }
                        let _ = weak.update(cx, |_, cx| cx.quit());
                    }
                    other => {
                        let _ = weak.update(cx, |this, cx| {
                            transition_after_shutdown_failure(&mut this.io_stop);
                            this.quitting = false;
                            this.shutdown_error = Some(format!("存檔尚未安全排空：{other:?}"));
                            this.desktop.show();
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(error) => {
                transition_after_shutdown_failure(&mut self.io_stop);
                self.quitting = false;
                self.shutdown_error = Some(format!("無法開始安全退出：{error}"));
                self.desktop.show();
            }
        }
        cx.notify();
    }

    fn cancel_quit_and_resume(&mut self, cx: &mut Context<Self>) {
        self.quitting = false;
        self.confirm_force_quit = false;
        self.shutdown_error = None;
        if self.io_stop.is_cancelled() {
            self.io_stop = tokio_util::sync::CancellationToken::new();
        }
        self.refreshing = false;
        self.live_busy = false;
        self.dynamic_busy = false;
        self.live.busy = false;
        self.settings.busy = false;
        for key in self.pending_images.drain().collect::<Vec<_>>() {
            self.pictures.pop(&key);
        }
        for feed in &mut self.feeds {
            feed.initialized = false;
        }
        self.videos.initialized = false;
        match self.services.clone() {
            Some(services) if services.stop.is_cancelled() => {
                self.loading = true;
                self.run(
                    async move { services.resume_after_shutdown().await },
                    cx,
                    |this, result, cx| {
                        this.loading = false;
                        match result {
                            Ok(services) => {
                                this.services = Some(services);
                                this.reload_snapshot(cx);
                                if this.view == View::Feed {
                                    this.load_feed(0, true, cx);
                                    if this.dual {
                                        this.load_feed(1, true, cx);
                                    }
                                } else if this.view == View::Videos {
                                    this.load_videos(true, cx);
                                }
                            }
                            Err(error) => this.error = Some(error.to_string()),
                        }
                        cx.notify();
                    },
                );
            }
            None => self.bootstrap(cx),
            Some(_) => {
                if self.view == View::Feed {
                    self.load_feed(0, true, cx);
                    if self.dual {
                        self.load_feed(1, true, cx);
                    }
                } else if self.view == View::Videos {
                    self.load_videos(true, cx);
                }
            }
        }
        cx.notify();
    }

    fn force_quit_now(&mut self, cx: &mut Context<Self>) {
        self.force_quit = true;
        cx.quit();
    }

    #[inline(never)]
    pub(super) fn shutdown_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .w(px(440.))
            .p_6()
            .rounded_lg()
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .flex()
            .flex_col()
            .gap_4();

        if let Some(error) = &self.shutdown_error {
            card = card
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child("安全退出未完成"),
                )
                .child(ui::notice_banner(Tone::Danger, error, Vec::new(), cx));

            let mut actions = div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap_2();
            actions = actions.child(
                Button::new("cancel-exit-resume")
                    .outline()
                    .label("返回應用")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_quit_and_resume(cx))),
            );
            if self.confirm_force_quit {
                actions = actions.child(
                    Button::new("force-exit-confirm")
                        .danger()
                        .label("確認強制退出")
                        .on_click(cx.listener(|this, _, _, cx| this.force_quit_now(cx))),
                );
            } else {
                actions = actions.child(
                    Button::new("force-exit-request")
                        .ghost()
                        .label("強制退出")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_force_quit = true;
                            cx.notify();
                        })),
                );
            }
            actions = actions.child(
                Button::new("retry-exit")
                    .primary()
                    .label("重試安全退出")
                    .on_click(cx.listener(|this, _, _, cx| this.request_quit(cx))),
            );
            card = card.child(actions);
        } else {
            card = card
                .items_center()
                .child(crate::ascii::working("shutdown-wait"))
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .child("正在安全退出"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_center()
                        .text_color(cx.theme().muted_foreground)
                        .child("正在停止下載與背景輪詢，並完成所有已接受的存檔寫入…"),
                );
        }

        div()
            .id("shutdown-overlay")
            .absolute()
            .inset_0()
            .bg(cx.theme().overlay)
            .flex()
            .items_center()
            .justify_center()
            .p_8()
            .child(card)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::prelude::v1::test;

    #[test]
    fn shutdown_failure_renews_cancelled_io_token() {
        let mut token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());

        let state = transition_after_shutdown_failure(&mut token);
        assert!(!state.quitting);
        assert!(state.io_accepting);
        assert!(state.show_overlay);
        assert!(!token.is_cancelled());
    }
}
