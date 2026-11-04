## rust-linux conformance

- build: a99d1494 (dirty), `/home/nick/develop/projects/my/ngterm/target/conformance-stage`
- mode: ratchet-only, the ids of `/home/nick/develop/projects/my/ngterm/conformance/rust-passing.txt` + --also tests/cli/test_cli_messages.py::test_send_read_ack_output + --also tests/cli/test_cli_messages.py::test_identity_environment + --also tests/cli/test_cli_messages.py::test_send_partial_failure + --also tests/cli/test_cli_messages.py::test_read_wait_timeout + --also tests/cli/test_cli_messages.py::test_read_ack_requires_confirmation + --also tests/cli/test_cli_output.py::test_live_run_job_uses_bundled_terminfo + --also tests/cli/test_cli_output.py::test_session_info_sets_and_clears + --also tests/cli/test_document_pages_cli.py::test_cli_document_sources_resolve_relative_paths + --also tests/cli/test_document_pages_cli.py::test_cli_document_sources_reject_incompatible_flags_before_connecting + --also tests/protocol/test_companions.py::test_duplicate_as_companion_rebinds_session_and_pane_ids + --also tests/protocol/test_document_pages.py::test_image_page_reads_back_its_source + --also tests/protocol/test_document_pages.py::test_markdown_page_reads_back_its_source + --also tests/protocol/test_document_pages.py::test_document_open_refusals + --also tests/protocol/test_document_pages.py::test_local_document_pages_are_not_remote_pages + --also tests/protocol/test_live_sessions.py::test_a_wait_command_keeps_scrollback_after_attached_exit + --also tests/protocol/test_live_sessions.py::test_remote_overlay_points_reach_capable_presenters + --also tests/protocol/test_live_sessions.py::test_older_presenter_gets_percent_without_points + --also tests/protocol/test_live_sessions.py::test_remote_page_points_and_resizes_reach_presenter + --also tests/protocol/test_live_sessions.py::test_group_attach_validates_before_remote_discovery + --also tests/protocol/test_live_sessions.py::test_tree_parent_membership_lists_companions_in_sidebar_order + --also tests/protocol/test_messages.py::test_send_read_ack + --also tests/protocol/test_messages.py::test_read_ack_is_atomic_and_bounded + --also tests/protocol/test_messages.py::test_message_event_has_recipient_location_and_no_body + --also tests/protocol/test_messages.py::test_message_limits_and_partial_rejections + --also tests/protocol/test_messages.py::test_broadcast_scope_and_identity + --also tests/protocol/test_messages.py::test_aliases_deliver_once + --also tests/protocol/test_messages.py::test_mailbox_follows_pane_identity + --also tests/protocol/test_messages.py::test_mailbox_lifecycle + --also tests/protocol/test_messages.py::test_restart_drops_mail_sent_during_shutdown + --also tests/protocol/test_session_info.py::test_info_set_reject_and_clear + --also tests/protocol/test_session_info.py::test_info_validation + --also tests/protocol/test_session_info.py::test_info_expiry_refresh_and_cancel + --also tests/protocol/test_session_info.py::test_info_not_persisted_or_duplicated + --also tests/protocol/test_session_info.py::test_info_soft_close_undo + --also tests/protocol/test_session_info.py::test_info_tree_changed + --also tests/ui/test_html_overlay.py::test_a_markdown_file_renders_and_the_tree_names_its_source + --also tests/ui/test_settings.py::test_session_info_display_picker_persists + --also tests/ui/test_settings.py::test_row_icons_hide_independently + --also tests/ui/test_sidebar_rows.py::test_session_info_renders_visible_text_below_its_row
- suite time: 1085s
- ngterm: `/home/nick/develop/projects/my/ngterm/target/conformance-stage/bin/ngterm`
- Vulkan: lavapipe only, `/home/nick/.cache/ngterm/vendor/lavapipe-13261478a29d822b/lvp_icd.json`
- host: Linux 7.3.1-1-cachyos-server, 2026-11-04T08:19:34Z
- layers: protocol cli behaviors ui
- display: headless
- pytest status: 1
- exit status: 0

```
rust: 986 pass, 2 fail, 0 skip(unsupported), 0 skip(not-yet-at-level), 0 skip(other), 0 reruns, parity 99.8%
FAILED conformance/tests/protocol/test_messages.py::test_restart_drops_mail_sent_during_shutdown
FAILED conformance/tests/ui/test_settings.py::test_session_info_display_picker_persists
===== 2 failed, 986 passed, 8 deselected, 1 warning in 1084.38s (0:18:04) ======
```

### ratchet

```
newly passing, not in the list (54):
tests/cli/test_cli_messages.py::test_identity_environment
tests/cli/test_cli_messages.py::test_read_ack_requires_confirmation[None]
tests/cli/test_cli_messages.py::test_read_ack_requires_confirmation[acked1]
tests/cli/test_cli_messages.py::test_read_ack_requires_confirmation[acked2]
tests/cli/test_cli_messages.py::test_read_wait_timeout
tests/cli/test_cli_messages.py::test_send_partial_failure
tests/cli/test_cli_messages.py::test_send_read_ack_output
tests/cli/test_cli_output.py::test_live_run_job_uses_bundled_terminfo[absent]
tests/cli/test_cli_output.py::test_live_run_job_uses_bundled_terminfo[empty]
tests/cli/test_cli_output.py::test_session_info_sets_and_clears
tests/cli/test_document_pages_cli.py::test_cli_document_sources_reject_incompatible_flags_before_connecting[--image]
tests/cli/test_document_pages_cli.py::test_cli_document_sources_reject_incompatible_flags_before_connecting[--markdown]
tests/cli/test_document_pages_cli.py::test_cli_document_sources_resolve_relative_paths[image]
tests/cli/test_document_pages_cli.py::test_cli_document_sources_resolve_relative_paths[markdown]
tests/protocol/test_companions.py::test_duplicate_as_companion_rebinds_session_and_pane_ids[live]
tests/protocol/test_companions.py::test_duplicate_as_companion_rebinds_session_and_pane_ids[plain]
tests/protocol/test_document_pages.py::test_document_open_refusals[image]
tests/protocol/test_document_pages.py::test_document_open_refusals[markdownFile]
tests/protocol/test_document_pages.py::test_image_page_reads_back_its_source
tests/protocol/test_document_pages.py::test_local_document_pages_are_not_remote_pages
tests/protocol/test_document_pages.py::test_markdown_page_reads_back_its_source
tests/protocol/test_live_sessions.py::test_a_wait_command_keeps_scrollback_after_attached_exit
tests/protocol/test_live_sessions.py::test_group_attach_validates_before_remote_discovery
tests/protocol/test_live_sessions.py::test_older_presenter_gets_percent_without_points[page]
tests/protocol/test_live_sessions.py::test_older_presenter_gets_percent_without_points[program]
tests/protocol/test_live_sessions.py::test_remote_overlay_points_reach_capable_presenters
tests/protocol/test_live_sessions.py::test_remote_page_points_and_resizes_reach_presenter
tests/protocol/test_live_sessions.py::test_tree_parent_membership_lists_companions_in_sidebar_order
tests/protocol/test_messages.py::test_aliases_deliver_once
tests/protocol/test_messages.py::test_broadcast_scope_and_identity
tests/protocol/test_messages.py::test_mailbox_follows_pane_identity
tests/protocol/test_messages.py::test_mailbox_lifecycle
tests/protocol/test_messages.py::test_message_event_has_recipient_location_and_no_body
tests/protocol/test_messages.py::test_message_limits_and_partial_rejections
tests/protocol/test_messages.py::test_read_ack_is_atomic_and_bounded
tests/protocol/test_messages.py::test_send_read_ack
tests/protocol/test_session_info.py::test_info_expiry_refresh_and_cancel
tests/protocol/test_session_info.py::test_info_not_persisted_or_duplicated
tests/protocol/test_session_info.py::test_info_set_reject_and_clear
tests/protocol/test_session_info.py::test_info_soft_close_undo
tests/protocol/test_session_info.py::test_info_tree_changed
tests/protocol/test_session_info.py::test_info_validation[args0-invalid info mode:  (set|clear)]
tests/protocol/test_session_info.py::test_info_validation[args1-session.info set requires text]
tests/protocol/test_session_info.py::test_info_validation[args2-session.info clear takes no text, markdown or hide-after]
tests/protocol/test_session_info.py::test_info_validation[args3-session.info clear takes no text, markdown or hide-after]
tests/protocol/test_session_info.py::test_info_validation[args4-session.info: info text is over 512 bytes]
tests/protocol/test_session_info.py::test_info_validation[args5-session.info: --hide-after must be 0...86400 seconds]
tests/protocol/test_session_info.py::test_info_validation[args6-session.info: info text is empty]
tests/protocol/test_session_info.py::test_info_validation[args7-session.info: info text must be one line]
tests/protocol/test_session_info.py::test_info_validation[args8-session.info: info text contains a control character]
tests/protocol/test_session_info.py::test_info_validation[args9-session.info: invalid info markdown; quote attribute values: ^[text](color: "green")]
tests/ui/test_html_overlay.py::test_a_markdown_file_renders_and_the_tree_names_its_source
tests/ui/test_settings.py::test_row_icons_hide_independently
tests/ui/test_sidebar_rows.py::test_session_info_renders_visible_text_below_its_row
ratchet holds: 932 listed id(s), ratchet-only run
```

### catalog coverage

ui ids exercised by a passing test: 123/129 (95.3%)
