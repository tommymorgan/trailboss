# Feature: Pi tabs show background subagent work

**Created**: 2026-11-09
**Goal**: A Pi tab whose parent turn has settled while pi-subagents background runs remain shows waiting, not idle, matching OMP.
> Plan home: repository
> Target repository: git@github.com:tommymorgan/trailboss.git
> Source PRD: none; direct bug report from Tommy (2026-11-09). No coverage ledger: no PRD acceptance criteria exist.
> Context: docs/adr/0008-pi-background-run-wait-state.md; crates/trailboss-runtime/src/integration/assets/omp/trailboss-agent-state.ts (`awaiting background work`); pi-subagents docs/extension-api.md "Herdr integration"
> Source base: d5e03003378fb3e8e4f6b784826d39d3eb6a3668

## Slice 1: Pi background wait state
**Tracer bullet**: A settled Pi root session receives `herdr:busy` active and reports `waiting` / `awaiting background work` to Trailboss; clearing it reports `idle`.
**Touches**: crates/trailboss-runtime/src/integration/assets/pi/trailboss-agent-state.ts, its Bun tests in assets/trailboss-agent-state.test.ts, PI_INTEGRATION_VERSION in integration/mod.rs.
**Off-limits**: OMP and other integration assets, Core state handling, pi-subagents, installed `~/.pi` files, the running daily Trailboss.

### Task 1.1: Report Pi background runs as waiting
<!-- estimate: ~40 source lines + ~150 test lines, high confidence | feature-files: features/pi-agent-status.feature | demoable: yes -->
> Depends on: none
> Shippable state: Pi integration v10 reports waiting during background runs with blocked and working precedence preserved; existing idle/working/blocked/session behavior unchanged. Installed v9 copies show as outdated until Tommy reinstalls.
> Boundary verification: Bun asset suite (all Pi and OMP tests) and `cargo test -p trailboss-runtime integration::` pass at the exact head; the recording-socket tests observe the scenario requests below.
> Build status: COMPLETE locally, bookmark pi-background-wait-slice-1-task-1. Not pushed; installed ~/.pi extension and daily Trailboss untouched. Every commit head passed the Bun asset suite (57→61 tests, 0 fail, ~1.8s each). Final head: `cargo test -p trailboss-runtime --lib integration::` 183 passed (0.72s); clippy all-targets -D warnings 0 findings; rustfmt clean. Each new test failed under a mutation of the behavior it names. One bounded basic review: 0 defects; one test-name overclaim fixed.

<!-- TODO -->
# Living: features/pi-agent-status.feature::SettledPiWithBackgroundRunsReportsWaiting
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: SettledPiWithBackgroundRunsReportsWaiting
  Given a Pi TUI root session has reported idle
  When pi-subagents emits herdr:busy with active true and label "⏳ 1 subagent (worker)"
  Then Trailboss receives state "waiting" with message "awaiting background work"
  When pi-subagents emits herdr:busy with active false
  Then Trailboss receives state "idle"

<!-- TODO -->
# Living: features/pi-agent-status.feature::PiOwnTurnOutranksBackgroundWait
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: PiOwnTurnOutranksBackgroundWait
  Given a Pi TUI root session is waiting on background runs
  When the parent agent starts a turn
  Then Trailboss receives state "working"
  When the parent turn settles while the background runs remain
  Then Trailboss receives state "waiting" with message "awaiting background work"

<!-- TODO -->
# Living: features/pi-agent-status.feature::PiBlockedOutranksBackgroundWait
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: PiBlockedOutranksBackgroundWait
  Given a Pi TUI root session is waiting on background runs
  When herdr:blocked is raised with label "approval"
  Then Trailboss receives state "blocked" with message "approval"
  When herdr:blocked is cleared while the background runs remain
  Then Trailboss receives state "waiting" with message "awaiting background work"

<!-- TODO -->
# Living: features/pi-agent-status.feature::PiBackgroundLabelChangeSendsNoIdle
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: PiBackgroundLabelChangeSendsNoIdle
  Given a Pi TUI root session is waiting on background runs
  When pi-subagents emits herdr:busy active false and then active true in the same tick
  Then Trailboss receives no further state report

<!-- TODO -->
# Living: features/pi-agent-status.feature::PiReloadRestoresBackgroundWait
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: PiReloadRestoresBackgroundWait
  Given a Pi reload restores an active background run before the Trailboss extension's session_start runs
  When the Trailboss extension's session_start runs in TUI mode with the parent idle
  Then the first state Trailboss receives is "waiting" with message "awaiting background work"
