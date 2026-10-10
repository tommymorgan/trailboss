# Render widget-tree declarations through the public plugin API

Plugins emit row, column, label, leader, spacer and list data; each client renderer draws it under core-defined semantics. Stock sidebar and tab-strip visuals use the same public API as other plugins, without privileged rendering. References to existing pane Surfaces are renderer-neutral; plugin-supplied cell-grid regions are outside this spike. See ADR 0005 for the approved rendering-ownership clarification.

Accepted by Tommy on 2026-10-10; source: Kizn design record, revision 3. Number allocated in this new repository with no remotes or default branch; recheck against the remote default branch before the first PR.
