//! The page's element on Windows: the panel (`super::super::panel`) the app draws, and over its content area the
//! page's own native window, which `WebView2` draws and takes the pointer and keys in. The element lays the content
//! area out and tells the page where it is ([`Page::place`]), blocks the pointer from what lies under it, and follows
//! the keyboard: the element taking it (the deck, a click into the page) gives the web view the keyboard, losing it
//! hands the keyboard back to the window (`Page::sync_native`).

use std::rc::Rc;

use gpui_kit::{
    App, Context, FocusHandle, Focusable, HitboxBehavior, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, canvas, div, px,
};

use super::super::panel;
use super::native::Bounds;
use super::page::Page;

/// The page's element (module docs).
pub struct PageView {
    page: Rc<Page>,
    focus: FocusHandle,
    released: bool,
    /// A failed load's error as selectable text (upstream's `.textSelection(.enabled)`).
    error: Option<crate::selectable::Selectable>,
    _subscriptions: Vec<Subscription>,
}

impl PageView {
    /// The element of `page` in `window`, whose native handle is `parent`.
    pub fn new(page: Rc<Page>, parent: isize, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let focused = cx.on_focus(&focus, window, |view, _, _| view.focus_changed(true));
        let blurred = cx.on_blur(&focus, window, |view, _, _| view.focus_changed(false));
        let hide_shown = panel::observe_hide_shown(cx);
        page.set_element(cx.entity().downgrade(), window.window_handle(), parent);
        Self {
            page,
            focus,
            released: false,
            error: None,
            _subscriptions: vec![focused, blurred, hide_shown],
        }
    }

    /// The page shown.
    #[must_use]
    pub fn page(&self) -> &Rc<Page> {
        &self.page
    }

    /// The handle that takes the keyboard for the page.
    #[must_use]
    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    fn focus_changed(&mut self, focused: bool) {
        if self.released {
            return;
        }
        self.page.set_focused(focused);
        if focused {
            self.page.focused();
        }
    }

    /// The page is gone: nothing more is drawn.
    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.released = true;
        cx.notify();
    }

    /// The content area: where the native window goes, under a hitbox that keeps the pointer from what lies beneath;
    /// off screen, nothing that takes the pointer.
    fn content(&self) -> gpui_kit::AnyElement {
        let page = Rc::clone(&self.page);
        let shown = self.page.on_screen() && !self.released;
        canvas(
            move |bounds, window, _| {
                page.place(Bounds {
                    x: f32::from(bounds.origin.x),
                    y: f32::from(bounds.origin.y),
                    width: f32::from(bounds.size.width),
                    height: f32::from(bounds.size.height),
                });
                shown.then(|| window.insert_hitbox(bounds, HitboxBehavior::BlockMouse))
            },
            |_, _, _, _| {},
        )
        .size_full()
        .into_any_element()
    }

    /// A failed load's error as selectable text, kept in step with the model's; `None` while the page did not fail.
    fn error_text(
        &mut self,
        overlay: &ngterm_core::html::HtmlOverlay,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui_kit::AnyElement> {
        let error = overlay
            .load_error
            .as_deref()
            .filter(|_| overlay.load_state == ngterm_core::html::HtmlLoadState::Failed)?;
        match &self.error {
            Some(shown) => shown.set(error, window, cx),
            None => self.error = Some(crate::selectable::Selectable::lines(error, window, cx)),
        }
        let colors = self.page.colors();
        self.error
            .as_ref()
            .map(|shown| shown.element("htmlOverlay.error-text", colors.ink(1.0), px(12.), false))
    }
}

impl Focusable for PageView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.page.drawn_in(window.window_handle());
        let overlay = self.page.overlay();
        let colors = self.page.colors();
        let poster = self.page.poster().clone();
        let content = self.content();
        let error = self.error_text(&overlay, window, cx);
        div()
            .id(SharedString::from(format!("html-page-{}", overlay.id)))
            .debug_selector(|| "html-page".to_owned())
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.background())
            .children(panel::strip(&overlay, &colors, &poster, panel::hide_shown(cx)))
            .child(
                div()
                    .id("htmlOverlay.page")
                    .debug_selector(|| "htmlOverlay.page".to_owned())
                    .accessibility_id("htmlOverlay.page")
                    .role(gpui_kit::Role::Document)
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .track_focus(&self.focus)
                    .child(content)
                    .children(panel::failure(&overlay, &colors, error)),
            )
    }
}
