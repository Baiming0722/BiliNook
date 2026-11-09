//! 使用獨立 WebView 資料目錄的播放器整合驗證工具。
#[path = "../src/player.rs"]
#[allow(dead_code)]
mod player;
use gpui::{prelude::*, *};
use std::{cell::RefCell, rc::Rc, time::Duration};
struct Probe {
    host: Rc<RefCell<player::PlayerHost>>,
    ids: Vec<u64>,
    covered: bool,
    fullscreen: bool,
}
impl Render for Probe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.host.borrow_mut().begin_frame();
        let mut grid = div().flex().flex_wrap().gap_2().size_full();
        for id in &self.ids {
            let host = self.host.clone();
            let id = *id;
            grid = grid.child(
                div().w(px(360.)).h(px(203.)).bg(rgb(0x151b26)).child(
                    canvas(
                        move |bounds, window, _| {
                            host.borrow_mut().layout(
                                id,
                                bounds,
                                window.content_mask().bounds,
                                window.scale_factor(),
                            );
                        },
                        |_, _, _, _| {},
                    )
                    .size_full(),
                ),
            );
        }
        div()
            .size_full()
            .bg(rgb(0x10141c))
            .text_color(white())
            .flex()
            .flex_col()
            .p_4()
            .gap_3()
            .child(
                div()
                    .id("cover-players")
                    .tab_index(0)
                    .cursor_pointer()
                    .p_3()
                    .bg(rgb(0x283345))
                    .child(if self.covered {
                        "恢復播放器"
                    } else {
                        "驗證面板遮擋"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.covered = !this.covered;
                        this.host.borrow_mut().cover(this.covered);
                        cx.notify();
                    })),
            )
            .child(grid)
    }
}
fn main() -> anyhow::Result<()> {
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
    }
    let ids: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|arg| arg.parse().ok())
        .take(9)
        .collect();
    anyhow::ensure!(!ids.is_empty(), "請傳入 1–9 個公開直播房號");
    gpui::application().run(move |cx| {
        gpui::init(cx);
        gpui::open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("直播播放器驗證".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(850.)),
                    cx,
                ))),
                ..Default::default()
            },
            cx,
            move |window, cx| {
                cx.new(move |cx: &mut Context<Probe>| {
                    let mut host = player::PlayerHost::new(
                        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../.local/live-port/player-profile"),
                    );
                    let mut start_worker = false;
                    for id in &ids {
                        match host.request(*id, true, window) {
                            Ok(start) => start_worker |= start,
                            Err(error) => eprintln!("建立失敗：{error:#}"),
                        }
                    }
                    let host = Rc::new(RefCell::new(host));
                    if start_worker {
                        let creating = host.clone();
                        cx.spawn(async move |weak, cx| {
                            loop {
                                let job = { creating.borrow_mut().next_job() };
                                let Some(job) = job else { break };
                                let result = job.run();
                                if let Some((id, error)) = creating.borrow_mut().finish_job(result)
                                {
                                    eprintln!("播放器 {id} 建立失敗：{error}");
                                }
                                if weak.update(cx, |_, cx| cx.notify()).is_err() {
                                    break;
                                }
                            }
                        })
                        .detach();
                    }
                    let polling = host.clone();
                    let handle = window.window_handle();
                    cx.spawn(async move |weak, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(120))
                                .await;
                            if weak
                                .update(cx, |this, cx| {
                                    for event in polling.borrow().events() {
                                        match event {
                                            player::PlayerEvent::Snapshot(id, bytes) => {
                                                eprintln!("Snapshot {id}: {} bytes", bytes.len());
                                            }
                                            player::PlayerEvent::Fullscreen(_, active) => {
                                                if this.fullscreen != active {
                                                    this.fullscreen = active;
                                                    let _ = handle.update(cx, |_, window, _| {
                                                        window.toggle_fullscreen()
                                                    });
                                                }
                                            }
                                            other => eprintln!("{other:?}"),
                                        }
                                    }
                                    cx.notify();
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                    Probe {
                        host,
                        ids,
                        covered: false,
                        fullscreen: false,
                    }
                })
            },
        )
        .expect("建立播放器驗證視窗失敗");
        cx.activate(true);
    });
    Ok(())
}
