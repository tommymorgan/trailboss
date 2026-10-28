# Trailboss

A fork of [herdr](https://github.com/herdrdev/herdr) — the runtime my coding
agents live on, shaped the way I run them.

This fork is maintained for my own workflow first, and published in case the
pieces are useful to anyone else. Upstream [herdr](https://herdr.dev) remains
the supported product; expect this line to diverge freely.


<p align="center">
  <img src="docs/assets/trailboss-demo.png" alt="Trailboss with its workspace and agent sidebar beside an active OMP conversation" width="820">
</p>

## Why my own fork

I run a couple dozen coding agents in one long-lived session, attach to them
from whatever machine I am at, and steer them from outside the terminal. Two
architectural directions mattered enough to own the runtime for:

- **Plugin-first UI architecture** — the sidebar and pane widgets are plugins
  over core projections, not hardcoded TUI screens. The interface can be
  replaced piece by piece without touching the core, which is how I want every
  UI surface here to grow.
- **Renderer-neutral core contracts** — declarations and projections instead
  of terminal-only state, so GUI and web clients can arrive later without a
  rewrite.

On top of those, changes I wanted to make quickly and dogfood daily without
asking a public project to adopt my workflow:

- **Structured control socket** — drive running agents programmatically:
  `chat_submit` (text, images, files), `turn_abort`, `agent.shutdown`, with
  idempotency, attachment validation, and typed rejections.
- **Remote attach** — connect from a laptop to the same core session over SSH,
  with live handoffs that swap the server binary without dropping clients or
  agents.

Everything here is tested against real agent sessions before it lands; the
machine I am typing on is usually running the previous commit.

## CLI help

Run `trailboss --help` (or `trailboss help`) for local/SSH attach options
and available commands. Use `trailboss help agent` or
`trailboss agent restart-idle --help` for command-specific guidance.
Help works without a running core or an interactive terminal.
The inherited upstream updater and update-channel commands are unavailable;
they must not replace a Trailboss installation with an upstream binary.

Trailboss keeps the `HERDR_*` integration environment and existing socket
overrides compatible with running agents. New session files use the
`trailboss` namespace; changing product labels does not migrate an existing
session or its socket. See [runtime provenance](docs/runtime-provenance.md#trailboss-naming-cutover).

## Moving a running tab between spaces

Use `trailboss tab move <tab-id> --workspace <space-id>` to move a complete
running tab to another space in the same Core. The tab is appended by default;
`--index N` chooses a zero-based insertion slot.
This command requires a Core that supports `tab.transfer`. An older Core returns
an error; the CLI does not fall back to reordering the source tab with `tab.move`.

The move retains the split layout, pane labels, zoom, focused pane and live
terminals. An attached client displaying the moved tab follows it to its new
space. The public API is `tab.transfer` with `tab_id`, `workspace_id` and an
optional `insert_index`; its `transfer` response includes the destination tab
and mappings from previous pane locators to their new locators.
Completed cross-space moves publish `tab.transferred`, followed by the existing
workspace and pane movement notifications. Subscribed stores refresh the source
and destination from the completed Core snapshot.
Old pane locators continue to target the same live panes until those panes
close, including across a supported live Core handoff. Existing shell and agent
environment variables are not rewritten. A handoff carrying these aliases
requires the replacement Core to acknowledge their restoration before taking
ownership; an alias-unaware replacement is rejected without committing the handoff.
On Linux, the importing Core also recovers a missing inherited locator from the
live shell attached to each imported PTY. It verifies the shell's terminal
session and process identity, preserves existing locators, and leaves ambiguous
or unreadable identities unresolved. This recovery does not rewrite environment
variables or restart shells or agents; other platforms retain normal alias
preservation without this Linux process-environment recovery.

Ordinary shutdown and restart restore the moved tab's destination, order, layout,
labels and selected pane, without recreating an emptied source space. This cold
restore starts new shell processes; it does not preserve running jobs or agents.
The existing `tab.move` API remains a reorder within one space.
This is not a transfer between different Core sessions.

The legacy handoff regressions use an explicitly supplied alias-blind older Core
with the same advertised version and protocol. They run only when requested and
cover both rejected alias-bearing import and successful alias-free legacy export:

```sh
TRAILBOSS_TEST_LEGACY_CORE=/absolute/path/to/alias-blind-trailboss \
  cargo nextest run --locked -p trailboss --test tab_transfer --run-ignored only \
  -E 'test(alias_blind_importer_) | test(alias_free_legacy_source_)'
```

## Command discovery for UI plugins

The socket API method `ui.command.list` takes empty parameters and returns the
current `boot_id`, `capabilities` and `commands`. It includes user-facing Core commands and
actions from enabled, platform-compatible plugin manifests; plugins do not
register a second command catalog.

Each command has a structured `key`, its original `title`, optional
`description`, context hints, a `target` and an opaque `fragment_name`.
Render an ordinary `Use` of that fragment from the matching live declaration
projection. Do not reconstruct action IDs from titles or dotted strings.
Resource commands belong under captured workspace/tab context; global and
client-local commands belong outside resource wrappers and popup ancestry.
Context hints are not an availability or permission check.
Manifest changes discovered by the catalog also republish generated fragments
to already-connected consumers. A title-only change retains the command's
boot-scoped fragment identity and original action owner.

Core publishes the interaction tree under its actual owner. Selection keeps
the existing native name, confirmation, focus and detach flows, or invokes
the original plugin action. Argument metadata uses optional JSON fields in the
existing generation-1 declaration protocol; the binary frame layout is unchanged.

An action can declare `argument_input = { kind = "raw_tail" }` and an optional
`argument_hint` in its manifest. The catalog retains the owner's original title
and hint. The matching endpoint projection carries bounded, display-safe metadata
for the issued command fragment. Both the catalog and endpoint welcome advertise
`plugin_action_arguments_v1`; missing fields remain compatible with legacy clients.

In the native palette, **Tab** accepts a fuzzy result and opens its raw-argument
editor without invoking the action. The canonical hint appears only when the
owner provides one. **Enter** executes the selected action: directly from search
with an explicit empty tail, or from the editor with its current value.
**Escape** cancels an unsubmitted draft. The editor retains its captured command
and destination rather than following later focus changes; observed owner,
destination or Core-boot invalidation prevents submission.

`plugin.action.invoke` accepts an optional `arguments` string for raw-tail actions.
Omitting it preserves legacy behavior; `""` explicitly supplies an empty tail.
Other action kinds reject a supplied value before dispatch. Spawned handlers get
the JSON-encoded string in `HERDR_PLUGIN_ARGUMENTS_JSON`; legacy invocations remove
that variable rather than inheriting it. Stream handlers receive the optional
top-level `arguments` string in their action envelope. Neither path changes the
manifest argv or parses the tail as shell arguments.

The native editor prevents duplicate submissions until the accepted owner's
command log reports success or failure. API acceptance alone does not complete
the interaction. A known API rejection or owner failure retains the draft for
correction and an explicit retry. An unknown outcome retains the draft and
blocks resubmission until the interaction is dismissed; it is never retried
automatically. Escape after dispatch dismisses local tracking without cancelling
the owner's work. A late result cannot overwrite a new draft.

A plugin action may set `confirmation_fragment` to one of its own ordinary
confirmation exports. The export must contain labels and exactly two terminal
rows: `host:cancel` and that action, with no target override or nested behavior.
Opening it does not invoke the action. Missing, invalid or ambiguous exports
leave the command listed with a `diagnostic` but no actionable generated
wrapper; consumers must not substitute a direct invocation.
The same unresolved behavior applies when a plugin declares a reserved Core
action ID: legacy registration remains intact, but discovery never issues a
plugin wrapper that would execute a different Core operation.

## Recent-tab switching

Press **F6** to switch immediately to the previously visited tab, including
tabs in other workspaces. **Shift+F6** cycles in the opposite direction.
Both keys traverse the same fixed snapshot of recent visits. After
**800 ms without either key**, switching finishes and only the final tab
moves to the front of the recent-tab order.

Normal typing, paste and clicks continue normally; Enter or Escape is not
required to finish. Closed tabs are skipped, and a later deliberate tab
selection cancels the pending cycle. History belongs to the attached client
and is not retained across detach/reattach.

The existing Ctrl-B, L switcher retains its explicit Enter/Escape finish.

## Ordinary CLI command status

The sidebar tracks commands in instrumented Bash panes, including remote
commands reached through SSH:

- Running commands show busy without being identified as coding agents.
- Exit code `0` shows completion; a nonzero code shows failure attention.
- Focusing the tab clears attention but retains `exit N` in its label.
- Missing or malformed exit codes remain unknown, never assumed successful.

New non-login Bash panes are instrumented automatically. For login shells
and SSH, run this on each host whose Bash sessions should report:

```sh
trailboss integration install shell
```

The updated Trailboss CLI must be available on that host; no remote Core or
background service is required. The installer writes
`${XDG_CONFIG_HOME:-$HOME/.config}/trailboss/shell-integration.bash` and adds
marked source blocks to `.bashrc` and the first existing login profile in
Bash's order: `.bash_profile`, `.bash_login`, `.profile`. It creates
`.bash_profile` only when none exists, so an existing lower-priority profile
is not shadowed. Existing prompt hooks and `PS0` are preserved.

New sessions load the helper through those startup files. To activate an
existing interactive Bash session after its current command has finished:

```sh
source "${XDG_CONFIG_HOME:-$HOME/.config}/trailboss/shell-integration.bash"
```

Remove the managed helper and source blocks with
`trailboss integration uninstall shell`. Other shell families remain
uninstrumented. Process probing can still detect an uninstrumented command
as busy, but cannot establish its exit result.

## OMP status ownership

Install the reporter on the host running OMP:

```sh
trailboss integration install omp
```

On Linux, a nested OMP process does not report pane status when its ancestry
contains another OMP process with the same pane and socket context. The walk
stops at the pane boundary, so an OMP process outside that pane does not
suppress its root reporter. Unreadable ancestry retains the existing behavior;
the `OMPCODE=1` nested-shell guard still applies on every platform.

Core also rejects conflicting OMP startup identities and their state reports
before advancing report sequence numbers. The root can return to Idle after
a rejected child's higher-sequence report. Same-session reloads, explicit
new/resume/fork transitions and journal moves between directories remain
supported.

Reporter updates apply when a process next loads the extension. Installing
an update does not reload or restart an existing OMP process.

## Thanks

Trailboss stands on [herdr](https://github.com/herdrdev/herdr) and its
contributors — the terminal runtime, agent management, and design that this
fork builds on and could not exist without. Thank you for making it and for
making it open.

## License

Apache-2.0, inherited from herdr. See [LICENSE](LICENSE).
