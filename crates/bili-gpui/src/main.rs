#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ascii;
mod assets;
mod audio;
mod desktop;
mod motion;
mod notifications;
mod player;
mod scroll;
mod ui;

use app::BiliApp;
use bili_core::{APP_NAME, runtime::CoreRuntime};
use gpui::*;

fn main() -> anyhow::Result<()> {
    // GPUI 的上層 DirectComposition target 會遮住 WebView2 子視窗。
    // 在任何執行緒啟動前選用官方提供的 DXGI HWND 路徑，仍保留 Direct3D 硬體加速。
    unsafe {
        std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1");
    }
    let directory = options()?;
    let Some(instance) = desktop::Instance::acquire(&directory)? else {
        return Ok(());
    };
    let runtime = CoreRuntime::start()?;
    gpui::application()
        .with_assets(assets::AppAssets)
        .run(move |cx: &mut App| {
            gpui::init(cx);
            if cfg!(debug_assertions)
                && std::env::var_os("BILI_REDUCE_MOTION").is_some_and(|v| v == "1")
            {
                cx.set_reduce_motion(true);
            }
            cx.set_app_identity(notifications::APP_ID, APP_NAME);
            BiliApp::bind_keys(cx);
            ui::apply_brand_theme("dark", cx);
            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            let runtime_for_view = runtime.clone();
            let (_, view) = gpui::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.), px(650.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(APP_NAME.into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let desktop =
                        desktop::Desktop::attach(instance, window).expect("無法初始化原生桌面服務");
                    cx.new(|cx| BiliApp::new(runtime_for_view, desktop, directory, window, cx))
                },
            )
            .expect("無法建立 GPUI 視窗");
            cx.on_app_quit(move |cx| {
                let entity = view.read(cx);
                entity.io_stop.cancel();
                let force_quit = entity.force_quit;
                let services = entity.services.clone();
                let runtime = runtime.clone();
                async move {
                    if !force_quit
                        && let Some(services) = services
                        && let Ok(result) = runtime.submit(async move { services.shutdown().await })
                    {
                        match result.recv().await {
                            Ok(Ok(())) => {}
                            other => eprintln!("[退出] 存檔排空失敗：{other:?}"),
                        }
                    }
                    runtime.shutdown().await;
                }
            })
            .detach();
            cx.activate(true);
        });
    Ok(())
}

fn options() -> anyhow::Result<std::path::PathBuf> {
    let mut directory = bili_core::store::data_dir()?;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            directory = args
                .next()
                .filter(|s| !s.is_empty())
                .map(Into::into)
                .ok_or_else(|| anyhow::anyhow!("--data-dir 需要目錄路徑"))?;
        } else if arg == "--print-data-dir" {
            println!("{}", directory.display());
            std::process::exit(0);
        } else if arg == "--help" || arg == "-h" {
            println!(
                "{APP_NAME}\n  --data-dir PATH  指定獨立資料目錄\n  --print-data-dir 顯示資料目錄後退出\n  Ctrl+1–4 分頁 · F5 刷新 · Ctrl+Q 安全退出"
            );
            std::process::exit(0);
        } else {
            anyhow::bail!("不支援的啟動參數");
        }
    }
    Ok(directory)
}
