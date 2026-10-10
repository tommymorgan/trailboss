# Spike validation

## Exit evidence

Run the Rust client against the vendored runtime, with the stock sidebar and tab strip supplied through the public plugin declaration path. Run Tommy’s actual session, not only synthetic fixtures. Validate attach from achy, image paste into the focused remote pane, session continuity and the ported agent tree. Record commands, revisions, host pair, terminal dimensions, pane workload and results. Do not collect pane content or credentials in shared logs.

## Latency gate

Gate name: `click-to-repaint-15-panes`. Measure locally and over achy→server SSH separately. Start at receipt of the physical click in the client; stop when the terminal presents the corresponding completed visual change, including the selected pane’s correct Surface and focus state. Do not stop at an unrelated paint, acknowledgement or optimistic highlight. Include transport and producer delays; measure visible terminal output, not only render-call duration.

Build-task measurement procedure: use 15 populated panes, a recorded repeatable workload and terminal size, and repeated workspace/tab/pane focus actions. Report every sample plus the maximum for each mode. The gate fails if any measured interaction exceeds 100ms; never replace the maximum with an average or percentile to obtain a pass. This procedure is an agent-selected task-local measurement method, not a promise about arbitrary WAN conditions. Acceptance uses Tommy’s actual achy remote setup.

Record a combined exit report with each gate and supporting evidence. Missing evidence is not a pass. A failed exit report directs work back to the existing herdr fork’s plugins; it does not automatically move sessions, rewrite repositories or terminate PTYs.
