//! The app's one button: a plain GPUI element (a stateful div) and no library widget, used by every button the app
//! draws: the title bar's, the window controls, the sidebar's footer and rows, the search bar's, Settings', the HTML
//! overlays' strip, the alerts' and the GUI ask's.
//!
//! A [`Button`] carries a button's accessibility (its role, its id, its name), its debug selector, a press that stays
//! the button's (no window move, no row click or drag, no focus change behind it), the chrome's hover and pressed
//! washes ([`Button::wash`]) or fills of the caller's, a toggled state, a dimmed and inert disabled state, and a hover
//! text in a hint of its own. It opens a dropdown menu of the library's on a press ([`Button::menu`]).
//!
//! The library's own `Button` is not used: its tooltips go through an overlay of the library's window root, which
//! ngterm's windows do not have, and its ghost hover is a wash flattened over the window body, which reads as almost
//! nothing over the sidebar's fill.

use std::rc::Rc;

use gpui_kit::component::Selectable;
use gpui_kit::component::menu::{DropdownMenu, PopupMenu};
use gpui_kit::{
    Anchor, App, AppContext as _, ClickEvent, Context, Div, ElementId, Hsla, InteractiveElement, Interactivity,
    IntoElement, MouseButton, ParentElement, Render, RenderOnce, Role, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Toggled, Window, div, prelude::FluentBuilder as _, px,
};
use ngterm_core::chrome::ChromeColors;

use super::{DIVIDER_ALPHA, color};

/// The chrome text's opacity over a hovered button (the title bar's buttons' `0.12`).
pub const HOVER: f32 = 0.12;
/// The chrome text's opacity over a button held down, toggled on, or whose menu is open.
pub const PRESSED: f32 = 0.18;
/// A disabled button's opacity (upstream's 0.35).
pub const DISABLED_OPACITY: f32 = 0.35;

/// A hover text: `text` on `ground`, in `ink`, framed in `ink` at the dividers' opacity.
struct Hint {
    text: SharedString,
    ink: Hsla,
    ground: Hsla,
}

impl Render for Hint {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        #[expect(clippy::cast_possible_truncation, reason = "an opacity")]
        let border = self.ink.opacity(DIVIDER_ALPHA as f32);
        div()
            .px_2()
            .py_1()
            .rounded(px(5.))
            .border_1()
            .border_color(border)
            .bg(self.ground)
            .text_color(self.ink)
            .text_size(px(12.))
            .child(self.text.clone())
    }
}

type Click = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type MenuBuilder = Box<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;
type OpenChange = Box<dyn Fn(&bool, &mut Window, &mut App)>;

/// A button's dropdown menu: where it opens, what it holds, and what hears it open and close.
struct Menu {
    anchor: Anchor,
    build: MenuBuilder,
    on_open_change: Option<OpenChange>,
}

/// What a button does with the keyboard's focus.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keyboard {
    /// A menu button takes it (a press, assistive technology), out of the Tab order; other buttons leave it alone.
    Takes,
    /// A press leaves it where it is (a field or a page behind keeps it).
    Keeps,
    /// A menu button Tab reaches too.
    TabStop,
}

/// A button (module docs). Its content is its children; it centers them in a flex row.
#[derive(IntoElement)]
pub struct Button {
    base: Stateful<Div>,
    hover: Option<Hsla>,
    pressed: Option<Hsla>,
    hint: Option<(SharedString, Hsla, Hsla)>,
    disabled: bool,
    toggled: Option<bool>,
    open: bool,
    keyboard: Keyboard,
    on_click: Option<Click>,
    menu: Option<Menu>,
}

impl Button {
    /// A button named `id`: its element id, its accessibility id and its debug selector.
    #[must_use]
    pub fn new(id: impl Into<SharedString>) -> Self {
        let id = id.into();
        let selector = id.to_string();
        Self::with_element_id(id.clone())
            .accessibility_id(id)
            .debug_selector(move || selector)
    }

    /// A button with element id `id` and no accessibility id or debug selector of its own (an alert's unnamed one).
    #[must_use]
    pub fn with_element_id(id: impl Into<ElementId>) -> Self {
        Self {
            base: div()
                .id(id)
                .role(Role::Button)
                .flex()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .justify_center(),
            hover: None,
            pressed: None,
            hint: None,
            disabled: false,
            toggled: None,
            open: false,
            keyboard: Keyboard::Takes,
            on_click: None,
            menu: None,
        }
    }

    /// The chrome's look: `ink` at [`HOVER`] under the pointer and at [`PRESSED`] held down, toggled on or with its
    /// menu open, see-through over whatever the button sits on.
    #[must_use]
    pub fn wash(self, ink: Hsla) -> Self {
        self.hover_fill(ink.opacity(HOVER)).pressed_fill(ink.opacity(PRESSED))
    }

    /// The fill under the pointer.
    #[must_use]
    pub fn hover_fill(mut self, fill: Hsla) -> Self {
        self.hover = Some(fill);
        self
    }

    /// The fill held down, toggled on or with its menu open.
    #[must_use]
    pub fn pressed_fill(mut self, fill: Hsla) -> Self {
        self.pressed = Some(fill);
        self
    }

    /// The hover text `text` in the chrome's colors; it names the button too.
    #[must_use]
    pub fn hint(self, text: impl Into<SharedString>, colors: &ChromeColors) -> Self {
        self.hint_in(text, color(colors.foreground, 1.0), color(colors.background, 1.0))
    }

    /// The hover text `text` in `ink` on `ground`; it names the button too.
    #[must_use]
    pub fn hint_in(mut self, text: impl Into<SharedString>, ink: Hsla, ground: Hsla) -> Self {
        let text = text.into();
        self.base = self.base.aria_label(text.clone());
        self.hint = Some((text, ink, ground));
        self
    }

    /// Dimmed and inert while `disabled`: no hover, no click; the hover text still shows.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// A toggle, on or off: on shows the pressed fill, and the state is its accessibility value.
    #[must_use]
    pub fn toggled(mut self, on: bool) -> Self {
        self.toggled = Some(on);
        self
    }

    /// A press leaves the keyboard where it is (a field or a page behind keeps it).
    #[must_use]
    pub fn keeps_keyboard(mut self) -> Self {
        self.keyboard = Keyboard::Keeps;
        self
    }

    /// What a click runs (an enabled button's only).
    #[must_use]
    pub fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// A press opens `menu` at `anchor` (a press on the open button closes it).
    #[must_use]
    pub fn menu(
        mut self,
        anchor: Anchor,
        menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.menu = Some(Menu {
            anchor,
            build: Box::new(menu),
            on_open_change: None,
        });
        self
    }

    /// A menu button Tab reaches (Settings' pickers). Every menu button takes the keyboard focus, so assistive
    /// technology can move to it, and Enter or Space opens its menu while it holds it; one outside the Tab order is
    /// reached by a press only, which leaves the keyboard where it was otherwise (a terminal keeps Tab).
    #[must_use]
    pub fn in_tab_order(mut self) -> Self {
        self.keyboard = Keyboard::TabStop;
        self
    }

    /// What hears the menu ([`Self::menu`]) open (`true`) and close.
    #[must_use]
    pub fn on_menu_open_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        if let Some(menu) = &mut self.menu {
            menu.on_open_change = Some(Box::new(handler));
        }
        self
    }

    /// The button itself: its element, without a menu. `owns_press` keeps a press from the elements behind; a menu's
    /// trigger leaves it to the popover around it, which opens on it.
    fn element(self, owns_press: bool) -> Stateful<Div> {
        let Self {
            base,
            hover,
            pressed,
            hint,
            disabled,
            toggled,
            open,
            keyboard,
            on_click,
            menu: _,
        } = self;
        let shows_pressed = open || toggled == Some(true);
        let keeps_keyboard = keyboard == Keyboard::Keeps;
        let on_click = on_click.map(Rc::new);
        base.when_some(toggled, |button, on| {
            button.aria_toggled(if on { Toggled::True } else { Toggled::False })
        })
        .when(shows_pressed, |button| button.when_some(pressed, gpui_kit::Styled::bg))
        .when(disabled, |button| button.opacity(DISABLED_OPACITY))
        .when(!disabled, |button| {
            button
                .when_some(hover.filter(|_| !shows_pressed), |button, fill| {
                    button.hover(move |style| style.bg(fill))
                })
                .when_some(pressed, |button, fill| button.active(move |style| style.bg(fill)))
                .when_some(on_click, |button, handler| {
                    button.on_click(move |event, window, cx| handler(event, window, cx))
                })
        })
        .when(owns_press || keeps_keyboard, |button| {
            button.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if keeps_keyboard {
                    window.prevent_default();
                }
                if owns_press {
                    cx.stop_propagation();
                }
            })
        })
        .when_some(hint, |button, (text, ink, ground)| {
            button.tooltip(move |_, cx| {
                let text = text.clone();
                cx.new(|_| Hint { text, ink, ground }).into()
            })
        })
    }
}

/// A menu's trigger: the button, its press left to the popover around it, holding a focus of its own (the library's
/// popover opens its menu on Enter or Space while its trigger is focused).
#[derive(IntoElement)]
struct Trigger(Button);

impl RenderOnce for Trigger {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (hover, in_tab_order) = (self.0.hover, self.0.keyboard == Keyboard::TabStop);
        let focus = self.0.interactivity().element_id.clone().map(|id| {
            window
                .use_keyed_state(id, cx, |_, cx| cx.focus_handle())
                .read(cx)
                .clone()
                .tab_stop(in_tab_order)
        });
        self.0
            .element(false)
            .when_some(focus, |button, focus| button.track_focus(&focus))
            // the keyboard's focus shows as the pointer's hover does
            .when_some(hover, |button, fill| button.focus_visible(move |style| style.bg(fill)))
    }
}

impl Styled for Trigger {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}

impl InteractiveElement for Trigger {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.0.interactivity()
    }
}

impl Selectable for Trigger {
    fn selected(mut self, selected: bool) -> Self {
        self.0.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.0.open
    }
}

impl DropdownMenu for Trigger {}

impl RenderOnce for Button {
    fn render(mut self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Some(menu) = self.menu.take() else {
            return self.element(true).into_any_element();
        };
        let build = menu.build;
        let popover =
            Trigger(self).dropdown_menu_with_anchor(menu.anchor, move |popup, window, cx| build(popup, window, cx));
        match menu.on_open_change {
            Some(handler) => popover
                .on_open_change(move |open, window, cx| handler(open, window, cx))
                .into_any_element(),
            None => popover.into_any_element(),
        }
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Button {}

impl ParentElement for Button {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui_kit::AnyElement>) {
        self.base.extend(elements);
    }
}

#[cfg(test)]
mod tests;
