---
worth: yes
where: crates/ngterm-gpui/src/settings/draw.rs:613
added: 2026-11-03
---
# Settings switches and sliders publish a second, unnamed library node inside their own

Each Settings toggle and slider is a frame that carries the row's accessibility id: a checkbox (`toggle`) or a slider
(`slider_frame`) with its name, state, disabled state and actions. Inside it, gpui-kit 0.7.3 still draws its own
control, which publishes a node of its own: `gpui_base::Switch` as a switch named after the row, and
`gpui_base::Slider` as an unnamed slider with its own value, range and increment/decrement. So assistive technology
meets each control twice, once under the row's id and once as the library's nested node. The library's slider
increment and decrement also move the knob without the edit a drag makes, because `SliderState::set_value` emits no
`SliderEvent::Change`. The frame's own steps make the edit, so the row's id reaches a working slider.

Surfaced by the Task 6 review of `docs/plans/20261031-e2e-ui-and-coverage.md` (round 1, the "inert slider"
finding). The fixer made the frame a full slider (value, range, step, steps that send the setting's edit) instead of
patching the library. The reviewer's suggested fix puts the row's id, name and disabled state on the library's own
node through a library patch. gpui-base and gpui-component are not vendored, and `gpui_kit::component::slider::Slider`
offers no accessibility id or label pass-through, so that fix means vendoring both crates (about 9 MB of source) with a
patch script like `scripts/vendor-accesskit-atspi.sh`. Alternatively, a GPUI patch could let a frame mark its
subtree as presentational. Either is larger than a review fix.
