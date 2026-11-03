//! The controls as assistive technology reads them: upstream's checkbox, slider and popup button under each row's id.

use gpui_kit::Element as _;
use gpui_kit::accesskit::{Action as A11yAction, Node, Role, Toggled};
use ngterm_core::settings_model::{GeneralChange, SettingsChange};

use super::*;

fn look() -> Look {
    Look {
        colors: crate::chrome::default_colors(),
        edits: Edits::new(crate::state::detached_poster()),
    }
}

/// The accessibility node `element` publishes.
fn node_of(element: &Stateful<Div>) -> Node {
    let role = element.a11y_role().expect("a control is an accessibility node");
    let mut node = Node::new(role);
    element.write_a11y_info(&mut node);
    node
}

fn right_click_paste(on: bool) -> SettingsChange {
    SettingsChange::General(GeneralChange::RightClickPaste(on))
}

#[test]
fn a_toggle_reads_as_a_named_checkbox_with_its_state_and_a_press() {
    let on = node_of(&toggle(
        "settings-right-click-paste",
        "Right-click pastes",
        true,
        true,
        Rc::new(right_click_paste),
        &look(),
    ));
    assert_eq!(on.role(), Role::CheckBox);
    assert_eq!(on.author_id(), Some("settings-right-click-paste"));
    assert_eq!(on.label(), Some("Right-click pastes"));
    assert_eq!(on.toggled(), Some(Toggled::True));
    assert!(!on.is_disabled());
    assert!(on.supports_action(A11yAction::Click));
    let off = node_of(&toggle(
        "settings-right-click-paste",
        "Right-click pastes",
        false,
        false,
        Rc::new(right_click_paste),
        &look(),
    ));
    assert_eq!(off.toggled(), Some(Toggled::False));
    assert!(off.is_disabled(), "a toggle that needs another setting takes no input");
    assert!(!off.supports_action(A11yAction::Click));
}

#[test]
fn a_slider_reads_its_value_within_its_bounds_and_steps() {
    let key = SliderKey::BackgroundBlur;
    let (min, max) = key.bounds();
    let shown = node_of(&slider_frame((key, 7.0), "Blur", true, &look().edits));
    assert_eq!(shown.role(), Role::Slider);
    assert_eq!(shown.author_id(), Some(key.id()));
    assert_eq!(shown.label(), Some("Blur"));
    assert_eq!(shown.numeric_value(), Some(7.0));
    assert_eq!(shown.min_numeric_value(), Some(min));
    assert_eq!(shown.max_numeric_value(), Some(max));
    assert_eq!(shown.numeric_value_step(), Some(SliderKey::STEP));
    assert!(!shown.is_disabled());
    assert!(shown.supports_action(A11yAction::Increment));
    assert!(shown.supports_action(A11yAction::Decrement));
    let greyed = node_of(&slider_frame((key, 0.0), "Blur", false, &look().edits));
    assert!(greyed.is_disabled());
    assert!(!greyed.supports_action(A11yAction::Increment));
    assert!(!greyed.supports_action(A11yAction::Decrement));
}

#[test]
fn a_picker_reads_as_a_named_combo_box() {
    let picker = Picker::new(
        vec![("Off".to_owned(), false), ("On".to_owned(), true)],
        &true,
        right_click_paste,
    );
    let node = picker_button("settings-some-picker", "Some picker", picker, &look()).a11y_node();
    assert_eq!(node.role(), Role::ComboBox);
    assert_eq!(node.author_id(), Some("settings-some-picker"));
    assert_eq!(node.label(), Some("Some picker"));
}
