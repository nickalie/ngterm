use crate::platform_sys as platform;
pub use ngterm_wpe_client::input::*;

pub(crate) const fn event_type(kind: PointerKind) -> platform::WPEEventType {
    match kind {
        PointerKind::Enter => platform::WPE_EVENT_POINTER_ENTER,
        PointerKind::Leave => platform::WPE_EVENT_POINTER_LEAVE,
        PointerKind::Move => platform::WPE_EVENT_POINTER_MOVE,
        PointerKind::Down(_) => platform::WPE_EVENT_POINTER_DOWN,
        PointerKind::Up(_) => platform::WPE_EVENT_POINTER_UP,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modifiers_match_platform_abi() {
        assert_eq!(Modifiers::CONTROL.bits(), platform::WPE_MODIFIER_KEYBOARD_CONTROL);
        assert_eq!(Modifiers::SHIFT.bits(), platform::WPE_MODIFIER_KEYBOARD_SHIFT);
        assert_eq!(Modifiers::ALT.bits(), platform::WPE_MODIFIER_KEYBOARD_ALT);
        assert_eq!(Modifiers::META.bits(), platform::WPE_MODIFIER_KEYBOARD_META);
        assert_eq!(Modifiers::CAPS_LOCK.bits(), platform::WPE_MODIFIER_KEYBOARD_CAPS_LOCK);
        assert_eq!(Modifiers::BUTTON1.bits(), platform::WPE_MODIFIER_POINTER_BUTTON1);
        assert_eq!(Modifiers::BUTTON2.bits(), platform::WPE_MODIFIER_POINTER_BUTTON2);
        assert_eq!(Modifiers::BUTTON3.bits(), platform::WPE_MODIFIER_POINTER_BUTTON3);
        assert_eq!(Modifiers::BUTTON4.bits(), platform::WPE_MODIFIER_POINTER_BUTTON4);
        assert_eq!(Modifiers::BUTTON5.bits(), platform::WPE_MODIFIER_POINTER_BUTTON5);
    }
}
