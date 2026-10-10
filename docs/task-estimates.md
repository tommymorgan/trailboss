# Task estimates

Estimates are incremental reviewable lines, not elapsed time. Borrowed runtime code is included rather than hidden as generated output; lock/generated files are excluded. A scenario is never divided by plan-split. Use slice-split against its actual diff if it exceeds the soft cap.

Reference checkout: ../herdr. Runtime extraction sizes come from the accepted scout inventory in docs/design-source.md. Inline inspection verified the agent-tree manifest, projection, control envelopes, store transport and server clipboard staging. Counts: project.ts 105 lines; control.ts 45; transport.ts 185; clipboard_image.rs 147; client_transport.rs 2435. These are estimation analogues, not proof that extraction works.

Task 1.1 includes coarse runtime vendoring and a fresh client: ~30000 lines, low confidence. Task 2.1 includes the first widget renderer and first real agent-tree consumer: ~1350 lines, low confidence. They exceed the ~500 soft cap and remain scenario-atomic. Stock UI follows agent-tree so it cannot accidentally become the first real consumer.

Before API compatibility acceptance, the extraction review must verify that the server-side session API was reused verbatim, not rewritten to pass fixtures. ADR 0001 is the source-reuse contract; behavioral compatibility alone cannot prove that architectural fact. No permanent source-text test is requested.

- Task 1.1: ~30000 lines; demoable yes; low confidence; over soft cap.
- Task 1.2: ~350 lines; demoable yes.
- Task 2.1: ~1350 lines; demoable yes; low confidence; over soft cap.
- Task 2.2: ~500 lines; demoable yes.
- Task 3.1: ~300 lines; demoable yes; low confidence.
- Task 3.2: ~400 lines; demoable yes.
- Task 3.3: ~440 lines; demoable yes.
- Task 3.4: ~480 lines; demoable yes.
- Task 3.5: ~350 lines; demoable yes.
- Task 4.1: ~240 lines; demoable yes.
- Task 4.2: ~350 lines; demoable yes.
- Task 5.1: ~450 lines; demoable yes; low confidence.
- Task 5.2: ~450 lines; demoable yes.
- Task 6.1: ~400 lines; demoable yes.
- Task 6.2: ~340 lines; demoable yes.
