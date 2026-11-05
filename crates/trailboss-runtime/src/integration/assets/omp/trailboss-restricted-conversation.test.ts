import { afterEach, expect, test } from "bun:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createConversationAdapter, readRestrictedAppGrant } from "./trailboss-agent-state";

const cleanups: Array<() => void> = [];
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); });

test("zero-byte reads expose native binding metadata without reading history or advancing a cursor", async () => {
  let visits = 0;
  const adapter = createConversationAdapter(() => ({
    sessionManager: {
      getLeafId: () => "private-tip",
      getEntry: () => { visits++; throw new Error("private history must not be read"); },
    },
    conversationControl: {
      version: 1,
      snapshot: () => ({ conversation_id: "review-session", reset_entry_id: null,
        context_epoch: "context-1", work: { state: "not_working", fence: null } }),
      submit: () => { throw new Error("read cannot submit"); },
      abort: () => { throw new Error("read cannot abort"); },
    },
  }));
  const response = await adapter({ operation: "conversation_read", terminal_id: "review-terminal",
    request_id: "metadata-1", params: { terminal_id: "review-terminal", max_bytes: 0 } });
  expect(response.result).toMatchObject({
    binding: { terminal_id: "review-terminal", conversation_id: "review-session", reset_entry_id: null },
    context_epoch: "context-1", work: { state: "not_working", fence: null },
    mode: "unchanged", records: [], next_cursor: null, baseline_cursor: null, has_more: false,
  });
  expect(visits).toBe(0);
});

function restrictedFixture() {
  let session = "review-session", idle: boolean | undefined = true, pending = false;
  let calls = 0, aborts = 0, visits = 0, denyHistory = false;
  let leaf: string | null = "private-tip";
  const entries = new Map<string, { id: string; parentId: string | null; type: string;
    message?: { role: string; content: string } }>([["private-tip", { id: "private-tip", parentId: null, type: "message",
    message: { role: "user", content: "PRIVATE RECORDING NOT FOR THE APP" } }]]);
  let send = async (_text: string) => { start(); };
  const ctx = {
    hasUI: true,
    sessionManager: {
      getSessionId: () => session,
      getSessionFile: () => "/synthetic/review.jsonl",
      getLeafId: () => leaf,
      getEntry: (id: string) => {
        visits++;
        if (denyHistory) throw new Error("metadata attempted history access");
        return entries.get(id);
      },
    },
    isIdle: () => idle,
    hasPendingMessages: () => pending,
    abort: () => { aborts++; },
  };
  let grant: { terminal_id: string; conversation_id: string; session_path: string } | undefined = {
    terminal_id: "review-terminal", conversation_id: session, session_path: "/synthetic/review.jsonl",
  };
  const options = { readGrant: () => grant, sendUserMessage: (text: string) => { calls++; return send(text); } };
  const adapter = createConversationAdapter(() => ctx, options);
  cleanups.push(() => adapter.close?.());
  function start() { idle = false; adapter.observe?.("agent_start", {}, ctx); }
  function finish() { idle = true; adapter.observe?.("agent_end", {}, ctx); }
  adapter.observe?.("session_start", {}, ctx);
  const call = (operation: string, params: unknown) => adapter({ operation: `conversation_${operation}`,
    terminal_id: "review-terminal", request_id: "request-1", params });
  const metadata = () => call("read", { terminal_id: "review-terminal", max_bytes: 0 });
  return { adapter, ctx, options, call, metadata, start, finish,
    calls: () => calls, aborts: () => aborts, visits: () => visits,
    setIdle(value: boolean | undefined) { idle = value; },
    setPending(value: boolean) { pending = value; },
    setSend(value: typeof send) { send = value; },
    denyHistory() { denyHistory = true; },
    setGrant(value: typeof grant) { grant = value; },
    appendIdleSetting() {
      entries.set("thinking", { id: "thinking", parentId: leaf, type: "thinking_level_change" });
      leaf = "thinking";
    },
    appendToolEntry() {
      adapter.observe?.("tool_execution_start", {}, ctx);
      entries.set("tool-start", { id: "tool-start", parentId: leaf, type: "custom",
        message: { role: "assistant", get content(): string { throw new Error("private message text was inspected"); } } });
      leaf = "tool-start";
    },
    reset() {
      entries.set("reset", { id: "reset", parentId: leaf, type: "reset_boundary" });
      leaf = "reset";
      adapter.observe?.("session_compact", {}, ctx);
    },
  };
}

function appInput(metadata, input_id = "job-1", text = "Read opaque job job-1 through the scoped app MCP.") {
  return { binding: metadata.binding, reporter_epoch: metadata.reporter_epoch,
    context_epoch: metadata.context_epoch, input_id, text, reply_style: "expanded" };
}

test("explicit app opt-in provides metadata without native control or history access", async () => {
  const f = restrictedFixture(), visits = f.visits();
  f.denyHistory();
  const response = await f.metadata();
  expect(response.result).toMatchObject({ mode: "unchanged", records: [], work: { state: "not_working", fence: null } });
  expect(response.result.capabilities).toContain("restricted_app_bridge_v1");
  expect(response.result.capabilities).toContain("restricted_submit_v1");
  expect(response.result.capabilities).not.toContain("guarded_submit_v1");
  expect(response.result.capabilities).not.toContain("restricted_abort_v1");
  expect(f.visits()).toBe(visits);
  expect(f.calls()).toBe(0);
});

test("busy or unknown work after metadata read refuses app submission before public send", async () => {
  for (const busy of [false, undefined]) {
    const f = restrictedFixture(), input = appInput((await f.metadata()).result);
    f.setIdle(busy);
    const result = await f.call("submit", input);
    expect(result.error).toMatchObject({ reason: "agent_busy", invocation: "not_started" });
    expect(f.calls()).toBe(0);
  }
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  const submission = f.call("submit", input);
  f.setIdle(false);
  expect((await submission).error).toMatchObject({ reason: "agent_busy", invocation: "not_started" });
  expect(f.calls()).toBe(0);
});

test("app admission refuses queued input and never borrows native abort ownership", async () => {
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  f.setPending(true);
  expect((await f.call("submit", input)).error.invocation).toBe("not_started");
  f.setPending(false);
  expect((await f.call("submit", input)).result.status).toBe("accepted_by_reporter");
  const running = (await f.metadata()).result;
  expect(running.work.state).toBe("working");
  expect(running.capabilities).not.toContain("restricted_abort_v1");
  expect((await f.call("abort", { ...input, work_fence: "unrelated-fence" })).error.reason).toBe("ownership_unknown");
  f.finish();
  f.start();
  expect((await f.call("abort", { ...input, work_fence: "later-fence" })).error.reason).toBe("ownership_unknown");
  expect(f.aborts()).toBe(0);
});

test("concurrent app duplicates share one receipt while conflicts and other jobs cannot deliver", async () => {
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  const gate = Promise.withResolvers<void>();
  let delivered = "";
  f.setSend(async text => { delivered = text; f.start(); await gate.promise; });
  const first = f.call("submit", input), duplicate = f.call("submit", input);
  await Promise.resolve();
  expect((await f.call("input_status", input)).result.status).toBe("dispatching");
  expect((await f.call("submit", { ...input, text: "different opaque job" })).error.reason).toBe("idempotency_conflict");
  expect((await f.call("submit", { ...input, input_id: "other-job" })).error.reason).toBe("instruction_outstanding");
  gate.resolve();
  const a = await first, b = await duplicate;
  expect(a.result.receipt).toBe(b.result.receipt);
  expect(a.result.status).toBe("accepted_by_reporter");
  expect(a.result).not.toHaveProperty("entry_id");
  expect(delivered).toContain(input.text);
  expect(delivered).not.toContain("Voice format");
  expect(f.calls()).toBe(1);
  f.finish();
  expect((await f.metadata()).result.capabilities).toContain("restricted_submit_v1");
});

test("uncertain public send stays unknown and cannot unlock or replay just because idle is true", async () => {
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  f.setSend(async () => { throw new Error("send outcome unknown"); });
  expect((await f.call("submit", input)).error.invocation).toBe("unknown");
  expect((await f.call("input_status", input)).result.status).toBe("unknown");
  expect((await f.call("submit", input)).error.invocation).toBe("unknown");
  expect((await f.call("submit", { ...input, input_id: "another-job" })).error.reason).toBe("instruction_outstanding");
  expect((await f.metadata()).result.work.state).toBe("unknown");
  expect(f.calls()).toBe(1);
});

test("an app turn that has started keeps desktop session transitions refused until it settles", async () => {
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  expect((await f.call("submit", input)).result.status).toBe("accepted_by_reporter");
  for (const name of ["session_before_switch", "session_before_branch", "session_before_tree"]) {
    expect(f.adapter.observe(name, {}, f.ctx)).toEqual({ cancel: true });
  }
  f.finish();
  expect(f.adapter.observe("session_before_switch", {}, f.ctx)).toBeUndefined();
});

test("an app send that never starts a turn cannot lock session transitions or the app slot", async () => {
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  // No model, usage preflight, cost cap or Esc during setup: the public send returns, no turn starts.
  f.setSend(async () => {});
  expect((await f.call("submit", input)).result.status).toBe("accepted_by_reporter");
  expect((await f.metadata()).result.capabilities).not.toContain("restricted_submit_v1");
  for (const name of ["session_before_switch", "session_before_branch", "session_before_tree"]) {
    expect(f.adapter.observe(name, {}, f.ctx)).toBeUndefined();
  }
  // A permitted transition releases the unstarted reservation for the new context.
  f.adapter.observe("session_switch", {}, f.ctx);
  const switched = (await f.metadata()).result;
  expect(switched.capabilities).toContain("restricted_submit_v1");
  // Without a transition, the next observed settled turn also releases it.
  const g = restrictedFixture(), held = appInput((await g.metadata()).result);
  g.setSend(async () => {});
  expect((await g.call("submit", held)).result.status).toBe("accepted_by_reporter");
  expect((await g.call("submit", { ...held, input_id: "job-2" })).error.reason).toBe("instruction_outstanding");
  g.start();
  g.finish();
  expect((await g.metadata()).result.capabilities).toContain("restricted_submit_v1");
  expect(g.calls()).toBe(1);
});

test("exact target, reset and reporter changes refuse stale app mutations", async () => {
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  f.setGrant({ terminal_id: "other-terminal", conversation_id: "review-session", session_path: "/synthetic/review.jsonl" });
  expect((await f.call("submit", input)).error.reason).toBe("binding_mismatch");
  f.setGrant({ terminal_id: "review-terminal", conversation_id: "review-session", session_path: "/synthetic/review.jsonl" });
  f.reset();
  expect((await f.call("submit", input)).error.reason).toBe("binding_mismatch");
  const fresh = createConversationAdapter(() => f.ctx, f.options);
  cleanups.push(() => fresh.close?.());
  fresh.observe("session_start", {}, f.ctx);
  const changed = await fresh({ operation: "conversation_input_status", terminal_id: "review-terminal",
    request_id: "status-1", params: input });
  expect(changed.result).toMatchObject({ status: "unknown", reason: "reporter_changed" });
  expect(f.calls()).toBe(0);
});

test("removing explicit app opt-in fails closed instead of enabling a public-API fallback", async () => {
  const f = restrictedFixture(), input = appInput((await f.metadata()).result);
  f.setGrant(undefined);
  expect((await f.metadata()).error.reason).toBe("structured_unavailable");
  expect((await f.call("submit", input)).error.invocation).toBe("not_started");
  expect(f.calls()).toBe(0);
});

test("app grants require private files, exact process incarnation and the existing conversation", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "repeater-grant-"));
  const file = path.join(directory, `${process.pid}.restricted-app.json`);
  const proc = fs.readFileSync("/proc/self/stat", "utf8");
  const grant = { version: 1, pid: process.pid,
    proc_start_ticks: proc.slice(proc.lastIndexOf(")") + 2).split(/\s+/)[19],
    terminal_id: "review-terminal", conversation_id: "review-session", session_path: "/synthetic/review.jsonl" };
  const ctx = { hasUI: true, sessionManager: {
    getSessionId: () => "review-session", getSessionFile: () => "/synthetic/review.jsonl" } };
  const write = value => { fs.writeFileSync(file, JSON.stringify(value), { mode: 0o600 }); fs.chmodSync(file, 0o600); };
  try {
    fs.chmodSync(directory, 0o700);
    write(grant);
    expect(readRestrictedAppGrant(ctx, directory)).toMatchObject(grant);
    for (const invalid of [{ ...grant, pid: process.pid + 1 }, { ...grant, proc_start_ticks: "0" },
      { ...grant, conversation_id: "replacement" }, { ...grant, session_path: "/other/session.jsonl" }]) {
      write(invalid);
      expect(readRestrictedAppGrant(ctx, directory)).toBeUndefined();
    }
    write(grant);
    fs.chmodSync(file, 0o644);
    expect(readRestrictedAppGrant(ctx, directory)).toBeUndefined();
    fs.unlinkSync(file);
    fs.symlinkSync("/dev/null", file);
    expect(readRestrictedAppGrant(ctx, directory)).toBeUndefined();
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("a late per-process grant can use startup metadata without a history bootstrap read", async () => {
  const f = restrictedFixture();
  f.setGrant(undefined);
  const fresh = createConversationAdapter(() => f.ctx, f.options);
  cleanups.push(() => fresh.close?.());
  fresh.observe("session_start", {}, f.ctx);
  f.setGrant({ terminal_id: "review-terminal", conversation_id: "review-session", session_path: "/synthetic/review.jsonl" });
  const visits = f.visits();
  f.denyHistory();
  const response = await fresh({ operation: "conversation_read", terminal_id: "review-terminal",
    request_id: "late-grant", params: { terminal_id: "review-terminal", max_bytes: 0 } });
  expect(response.result).toMatchObject({ mode: "unchanged", records: [],
    binding: { conversation_id: "review-session", reset_entry_id: null } });
  expect(f.visits()).toBe(visits);
});

test("idle settings entries without extension hooks eventually refresh metadata without a history read", async () => {
  const f = restrictedFixture();
  f.appendIdleSetting();
  await Bun.sleep(400);
  const visits = f.visits();
  f.denyHistory();
  const response = await f.metadata();
  expect(response.result).toMatchObject({ records: [],
    binding: { conversation_id: "review-session", reset_entry_id: null },
    work: { state: "not_working" } });
  expect(f.visits()).toBe(visits);
});

test("a FIFO opt-in marker is refused without blocking the reporter process", async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "repeater-fifo-"));
  const module = path.join(import.meta.dir, "trailboss-agent-state.ts");
  const script = `
    import fs from "node:fs"; import path from "node:path";
    import { readRestrictedAppGrant } from ${JSON.stringify(module)};
    const dir=${JSON.stringify(directory)};
    const made=Bun.spawnSync(["mkfifo",path.join(dir,process.pid+".restricted-app.json")]);
    if(made.exitCode!==0)process.exit(2);
    const result=readRestrictedAppGrant({hasUI:true,sessionManager:{}},dir);
    process.stdout.write(JSON.stringify({rejected:result===undefined}));
  `;
  let proc;
  try {
    fs.chmodSync(directory, 0o700);
    proc = Bun.spawn([process.execPath, "-e", script], { stdout: "pipe", stderr: "pipe" });
    const timer = setTimeout(() => proc.kill(), 1500);
    const code = await proc.exited;
    clearTimeout(timer);
    expect(code).toBe(0);
    expect(JSON.parse(await new Response(proc.stdout).text())).toEqual({ rejected: true });
  } finally {
    if (proc && proc.exitCode === null) { proc.kill(); await proc.exited; }
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("metadata requested immediately after a busy tool append waits for independent cache readiness", async () => {
  const f = restrictedFixture();
  f.start();
  f.appendToolEntry();
  const response = await f.metadata();
  expect(response.result).toMatchObject({
    binding: { conversation_id: "review-session", reset_entry_id: null },
    mode: "unchanged", work: { state: "working", fence: null },
    records: [], next_cursor: null, baseline_cursor: null, has_more: false,
  });
  expect(f.calls()).toBe(0);
});

test("reset while metadata awaits an independent refresh refuses the old context", async () => {
  const f = restrictedFixture();
  f.start();
  f.appendToolEntry();
  const pending = f.metadata();
  await Promise.resolve();
  f.reset();
  expect((await pending).error).toMatchObject({ reason: "structured_unavailable", invocation: "not_started" });
  expect(f.calls()).toBe(0);
});

test("removed or changed process opt-in while metadata waits cannot release metadata", async () => {
  for (const grant of [undefined, { terminal_id: "replacement-terminal",
    conversation_id: "review-session", session_path: "/synthetic/review.jsonl" }]) {
    const f = restrictedFixture();
    f.start();
    f.appendToolEntry();
    const pending = f.metadata();
    await Promise.resolve();
    f.setGrant(grant);
    expect((await pending).error).toMatchObject({ reason: "structured_unavailable", invocation: "not_started" });
    expect(f.calls()).toBe(0);
  }
});

test("closing the adapter wakes pending metadata reads without serving a stale binding", async () => {
  const f = restrictedFixture();
  f.start();
  f.appendToolEntry();
  const pending = f.metadata();
  await Promise.resolve();
  f.adapter.close();
  expect((await pending).error).toMatchObject({ reason: "structured_unavailable", invocation: "not_started" });
  expect((await f.metadata()).error.reason).toBe("structured_unavailable");
});

test("metadata cache nonconvergence remains unavailable after the bounded readiness wait", async () => {
  const f = restrictedFixture();
  f.start();
  f.appendToolEntry();
  f.denyHistory();
  expect((await f.metadata()).error).toMatchObject({ reason: "structured_unavailable", invocation: "not_started" });
  expect(f.calls()).toBe(0);
}, 1500);
