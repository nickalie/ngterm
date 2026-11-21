---
worth: yes
where: conformance/tools/run-windows.sh:184
added: 2026-11-22
---
# run-windows.sh runs paths and node ids after `--` beside the full layers

`run-linux.sh` replaces the protocol, cli, behaviors and ui layers with the paths and node ids after `--`, and gives
the plugin `--narrowed`, so that the nodeid report is partial. `run-windows.sh` still gives pytest
`"${win_layers[@]}"` and then `"${extra[@]}"`. Thus `run-windows.sh -- tests/protocol/test_probe.py` runs the full
suite plus that file, the report says that the mode is full, and the ratchet judges every listed id.

Port the split of `run-linux.sh` (the option values, the path candidates, the exit 2 for a path that is not there,
`--narrowed` and the `partial` mode line in report.md). Convert each found path with `win` before the `::` node part,
not after it. A shared helper that both runners source keeps the option list in one place. Test the port on a Windows
CI runner or the VM. `run-windows.sh`'s header and conformance/README.md > Rust runner and ratchet record the
difference until then.

Found in the revmux review of task #780 (run-linux.sh ignores a node id after `--`), round 1.
