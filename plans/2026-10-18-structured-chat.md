# Feature: Structured chat-submit and turn-abort for running OMP agents

**Created**: 2026-10-18
**Goal**: Let API clients deliver a structured prompt to a running OMP agent natively and abort the current turn — no PTY injection — through the Trailboss reporter control socket.
> Source: delegated acceptance criteria from the Kizn agent web build (peer agent omp-a285c6c684e6f3f9f2fd65d9, mail 001792288993474-jrvjjsuj, relayed by Tommy 2026-10-18) and its ADR 0002 (docs/adrs/0002-structured-chat-events.md in /home/tommy/src/kizn-agent-web-build).
> Constraint (Tommy, 2026-10-18): no harness core changes, including OMP. All edits stay in this repository; OMP is used only through its public extension API.
> Context: [domain model](../CONTEXT.md)

Additive wire surface: new `Method` enum entries only; frozen serialized names (`herdr:omp`, `HERDR_*`) and every existing method keep working. Running agents are never restarted; agents whose loaded reporter predates chat ops return a per-agent `structured_unavailable` error. Attachments are a second unit.

## Slice 1: Structured submission and abort

**Tracer bullet**: submit a text prompt to an idle OMP agent through the API and observe it land as a native user message in the agent's session JSONL.
**Touches**: api schema (method params/results), app dispatch + control-socket client, OMP reporter asset (op switch, idempotency, capability reply), generated API schema artifact, tests.
**Off-limits**: PTY keystroke paths (`agent.prompt`, `agent.send_keys`, `queue_user_input_submission`), OMP core code, restart or hot-load of running agents, web/UI clients, Collab.

### Task 1.1: chat-submit native delivery with idempotency
<!-- estimate: ~380 reviewable lines | feature-files: features/structured-chat.feature | demoable: yes -->

<!-- TODO -->
# Living: features/structured-chat.feature::Submit a structured prompt that lands natively
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Submit a structured prompt that lands natively
  Given an idle OMP agent is running under Trailboss with the chat-capable reporter loaded
  When a client calls agent.chat_submit with that agent, a text prompt, and idempotency key "k1"
  Then the method returns accepted with a turn id
  And the prompt appears in the agent's session JSONL as a native user message
  And reply and tool events stream to subscribers exactly as a typed prompt would
  And no PTY input path (queue_user_input_submission) is used
  When the client retries the same call with idempotency key "k1"
  Then no second message is delivered and the same turn id is returned

### Task 1.2: abort the current turn without stopping the process
<!-- estimate: ~200 reviewable lines | feature-files: features/structured-chat.feature | demoable: yes -->

<!-- TODO -->
# Living: features/structured-chat.feature::Abort the current turn and keep chatting
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Abort the current turn and keep chatting
  Given an OMP agent under Trailboss is mid-turn
  When a client calls agent.turn_abort for that agent
  Then the in-flight turn stops and the OMP process stays alive
  And a subsequent agent.chat_submit is accepted and delivered

### Task 1.3: structured-unavailable for pre-update agents
<!-- estimate: ~160 reviewable lines | feature-files: features/structured-chat.feature | demoable: yes -->

<!-- TODO -->
# Living: features/structured-chat.feature::Report structured-unavailable per pre-update agent
# Action: creates
# Status: TODO
# Living updated: NO
Scenario: Report structured-unavailable per pre-update agent
  Given one agent whose loaded reporter has no control socket and one whose reporter predates chat ops
  When a client calls agent.chat_submit or agent.turn_abort for either agent
  Then each call fails with a structured_unavailable error naming that agent
  And neither agent is restarted or otherwise disturbed

## Slice 2: Attachments (deferred second unit; do not build in slice 1)

### Task 2.1: image-native and machine-file-reference attachments
<!-- estimate: ~300 reviewable lines | feature-files: features/structured-chat.feature | demoable: yes -->

<!-- TODO -->
# Living: features/structured-chat.feature::Attach images and machine file references
# Action: extends
# Status: TODO
# Living updated: NO
Scenario: Attach images and machine file references
  Given an idle OMP agent under Trailboss with the chat-capable reporter
  When a client calls agent.chat_submit with text plus an image attachment
  Then the image is delivered as native image content in the same user message
  When a client calls agent.chat_submit with a machine file reference attachment
  Then the agent receives the materialized local file path for its tools

## Notes

- OMP extension API used (public, no core changes): `pi.sendUserMessage(content, options)` — idle starts a turn, streaming queues as steer; `ctx.abort()`; images are native `ImageContent`.
- turn ids are minted by the reporter (OMP's sendUserMessage returns void) and correlated with the following agent_start report.
- Idempotency lives in the reporter (key → turn id, bounded map); it covers socket retries but not a reporter process restart, by design.
- Installed runtime moved to omp/18.12.0 since the peer's research pinned 18.11.3; verification targets the installed version and flags the change to the peer.
