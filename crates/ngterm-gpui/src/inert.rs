//! Hiding a layer from the pointer as well as from the screen. GPUI's `invisible` skips an element's paint but still
//! prepaints its children, and a hitbox is inserted at prepaint, so a hidden layer that occludes (an HTML page's error
//! or engine panel, a page's web view) kept taking clicks, drags and the wheel from whatever lies under it. [`hide`]
//! keeps the element `invisible` and prepaints it under an empty content mask, which a hit test clips every hitbox to:
//! the hidden layer is still laid out (a hidden terminal keeps its size, a hidden page its document) but nothing of it
//! is ever hit.

use gpui_kit::{
    AnyElement, App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Styled, Window, px, size,
};

/// `element`, drawn and hit-tested only while `hidden` is false.
#[must_use]
pub fn hide<E: Styled + IntoElement>(element: E, hidden: bool) -> Inert {
    let element = if hidden { element.invisible() } else { element };
    Inert {
        child: element.into_any_element(),
        inert: hidden,
    }
}

/// An element whose hitboxes are never hit while it is inert ([`hide`]).
pub struct Inert {
    child: AnyElement,
    inert: bool,
}

impl Element for Inert {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let mask = self.inert.then(|| ContentMask {
            bounds: Bounds::new(bounds.origin, size(px(0.), px(0.))),
        });
        window.with_content_mask(mask, |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
