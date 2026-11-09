use super::*;
use crate::{assets::AppIcon, ui};
use gpui::component::{IconName, Sizable as _};

gpui::actions!(image_preview, [NextImage, PreviousImage]);

pub(super) struct Preview {
    urls: Vec<String>,
    index: usize,
    runtime: CoreRuntime,
    cache: Arc<bili_core::images::ImageCache>,
    image: Option<Arc<Image>>,
    error: Option<String>,
    loading: bool,
    version: u64,
    focus: FocusHandle,
}

impl Preview {
    pub(super) fn new(
        urls: Vec<String>,
        index: usize,
        runtime: CoreRuntime,
        cache: Arc<bili_core::images::ImageCache>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let index = index.min(urls.len().saturating_sub(1));
        let mut this = Self {
            urls,
            index,
            runtime,
            cache,
            image: None,
            error: None,
            loading: false,
            version: 0,
            focus: cx.focus_handle(),
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.version += 1;
        let version = self.version;
        self.image = None;
        self.error = None;
        self.loading = true;
        let cache = self.cache.clone();
        let url = self.urls[self.index].clone();
        match self.runtime.submit(async move {
            let url = bili_core::images::original_image_url(&url)?;
            cache.get(&url).await
        }) {
            Ok(result) => {
                cx.spawn(async move |weak, cx| {
                    let result = result
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err(anyhow!("圖片服務未回應")));
                    let _ = weak.update(cx, |this, cx| {
                        if this.version != version {
                            return;
                        }
                        this.loading = false;
                        match result {
                            Ok(image) => this.image = Some(super::common::gui_image(&image)),
                            Err(error) => this.error = Some(error.to_string()),
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Err(error) => {
                self.loading = false;
                self.error = Some(error.to_string());
            }
        }
    }

    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.urls.len() < 2 {
            return;
        }
        self.index = (self.index as isize + step).rem_euclid(self.urls.len() as isize) as usize;
        self.load(cx);
        cx.notify();
    }
}

impl Focusable for Preview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = (f32::from(window.viewport_size().height) - 240.).clamp(200., 640.);
        let content = if let Some(image) = &self.image {
            div()
                .w_full()
                .h(px(height))
                .rounded_md()
                .bg(cx.theme().background)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    img(image.clone())
                        .w_full()
                        .h(px(height))
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element()
        } else if self.loading {
            div()
                .w_full()
                .h(px(height))
                .rounded_md()
                .bg(cx.theme().background)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(crate::ascii::connecting("image-preview-wait"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("正在載入高畫質原圖…"),
                )
                .into_any_element()
        } else {
            div()
                .w_full()
                .h(px(height))
                .rounded_md()
                .bg(cx.theme().background)
                .child(ui::empty_state(
                    AppIcon::Image,
                    "原圖無法載入",
                    self.error.as_deref().unwrap_or("請檢查網路連線後重試"),
                    None,
                    cx,
                ))
                .into_any_element()
        };

        div()
            .key_context("ImagePreview")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap_3()
            .on_action(cx.listener(|this, _: &NextImage, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousImage, _, cx| this.step(-1, cx)))
            .child(content)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("preview-previous")
                                    .small()
                                    .outline()
                                    .icon(IconName::ChevronLeft)
                                    .label("上一張")
                                    .disabled(self.urls.len() < 2)
                                    .on_click(cx.listener(|this, _, _, cx| this.step(-1, cx))),
                            )
                            .child(
                                Button::new("preview-next")
                                    .small()
                                    .outline()
                                    .icon(IconName::ChevronRight)
                                    .label("下一張")
                                    .disabled(self.urls.len() < 2)
                                    .on_click(cx.listener(|this, _, _, cx| this.step(1, cx))),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} / {}{}",
                                self.index + 1,
                                self.urls.len(),
                                if self.urls.len() > 1 {
                                    " · ← → 切換 · Esc 關閉"
                                } else {
                                    " · Esc 關閉"
                                }
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("preview-retry")
                                    .small()
                                    .ghost()
                                    .icon(IconName::RefreshCw)
                                    .label("重試")
                                    .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                            )
                            .child(
                                Button::new("preview-close")
                                    .small()
                                    .label("關閉")
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            ),
                    ),
            )
    }
}
