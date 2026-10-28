import { expect, test } from "bun:test";
import { createConversationAdapter } from "./trailboss-agent-state";

type Entry = { id: string; parentId: string | null; type: string; message?: { role: string; content: string | Array<{ type: string; text?: string }>; stopReason?: string; synthetic?: boolean; attribution?: string } };

function fixture() {
  const entries = new Map<string, Entry>();
  let leaf: string | null = null;
  let contextEpoch = "context-1";
  let reset: string | null = null;
  let calls = 0;
  let aborts = 0;
  let submit = async (_request: unknown) => { calls++; return { status: "accepted" }; };
  const ctx = {
    sessionManager: { getLeafId: () => leaf, getEntry: (id: string) => entries.get(id) },
    conversationControl: {
      version: 1,
      snapshot: () => ({ conversation_id: "session-1", reset_entry_id: reset, context_epoch: contextEpoch, work: { state: "working", fence: "work-1" } }),
      submit: (request: unknown) => submit(request),
      abort: async (request: { work_fence: string }) => { if (request.work_fence !== "work-1") return { status: "rejected", reason: "work_changed", invocation: "not_started" }; aborts++; return { status: "acknowledged" }; }
    }
  };
  const adapter = createConversationAdapter(() => ctx);
  const call = (operation: string, params: unknown) => adapter({ operation: `conversation_${operation}`, terminal_id: "term-1", request_id: "req-1", params });
  const read = (cursor: string | null = null, max_bytes = 4096) => call("read", { terminal_id: "term-1", expected_context: cursor ? { conversation_id: "session-1", reset_entry_id: reset } : null, cursor, max_bytes });
  return { adapter, call, read, ctx,
    append(entry: Omit<Entry, "parentId">) { entries.set(entry.id, { ...entry, parentId: leaf }); leaf = entry.id; },
    setEpoch(epoch: string) { contextEpoch = epoch; },
    clear() { reset = "reset-1"; entries.set(reset, { id: reset, parentId: leaf, type: "reset_boundary" }); leaf = reset; contextEpoch = "context-clear"; },
    setSubmit(next: typeof submit) { submit = next; },
    calls: () => calls, aborts: () => aborts
  };
}

function submitParams(readResult, input_id = "input-1", text = "hello") {
  return { binding: readResult.binding, reporter_epoch: readResult.reporter_epoch,
    context_epoch: readResult.context_epoch, input_id, text, reply_style: "brief" };
}

test("silent baseline excludes tools, thinking and synthetic user injections but observes appended native answers", async () => {
  const f = fixture();
  f.append({ id: "user-1", type: "message", message: { role: "user", content: "question" } });
  f.append({ id: "tool-1", type: "message", message: { role: "toolResult", content: "SECRET_TOOL" } });
  f.append({ id: "synthetic-1", type: "message", message: { role: "user", content: "SECRET_INJECTION", synthetic: true } });
  f.append({ id: "answer-1", type: "message", message: { role: "assistant", stopReason: "stop", content: [{ type: "thinking", text: "SECRET_THINKING" }, { type: "text", text: "answer" }] } });
  const history = (await f.read()).result;
  expect(history.mode).toBe("snapshot");
  expect(history.records.map(record => record.text)).toEqual(["question", "answer"]);
  expect(JSON.stringify(history)).not.toContain("SECRET");
  expect((await f.read(history.baseline_cursor)).result.mode).toBe("unchanged");
  f.append({ id: "answer-2", type: "message", message: { role: "assistant", stopReason: "stop", content: [{ type: "text", text: "new answer" }] } });
  const delta = (await f.read(history.baseline_cursor)).result;
  expect(delta.mode).toBe("delta");
  expect(delta.records.map(record => [record.entry_id, record.text, record.has_tool_calls])).toEqual([["answer-2", "new answer", false]]);
});

test("escaped multibyte replies paginate without truncation and remain pinned while newer messages arrive", async () => {
  const f = fixture(), original = '🙂"\\\n'.repeat(1800);
  f.append({ id: "large", type: "message", message: { role: "assistant", stopReason: "stop", content: [{ type: "text", text: original }] } });
  let response = await f.read();
  const baseline = response.result.baseline_cursor;
  let assembled = "", offset = 0, pages = 0;
  f.append({ id: "later", type: "message", message: { role: "assistant", stopReason: "stop", content: [{ type: "text", text: "later" }] } });
  while (true) {
    const result = response.result;
    expect(Buffer.byteLength(JSON.stringify({ id: "req-1", result: { type: "agent_conversation", result } })) + 1).toBeLessThanOrEqual(4096);
    for (const record of result.records) {
      expect(record.entry_id).toBe("large");
      expect(record.text_offset_bytes).toBe(offset);
      expect(record.text_total_bytes).toBe(Buffer.byteLength(original));
      assembled += record.text; offset += Buffer.byteLength(record.text);
      expect(record.text).not.toContain("�");
    }
    pages++;
    if (!result.has_more) break;
    response = await f.read(result.next_cursor);
    expect(pages).toBeLessThan(100);
  }
  expect(assembled).toBe(original);
  expect((await f.read(baseline)).result.records.map(record => record.entry_id)).toEqual(["later"]);
});

test("all-hidden suffix advances the cursor and a branch switch away and back invalidates the old epoch", async () => {
  const f = fixture(), initial = (await f.read()).result;
  f.append({ id: "hidden", type: "message", message: { role: "toolResult", content: "tool" } });
  const hidden = (await f.read(initial.baseline_cursor)).result;
  expect(hidden.records).toEqual([]);
  expect(hidden.next_cursor).not.toBe(initial.baseline_cursor);
  expect((await f.read(hidden.next_cursor)).result.mode).toBe("unchanged");
  f.setEpoch("context-away-back");
  const switched = (await f.read(hidden.next_cursor)).result;
  expect(switched.mode).toBe("reset");
  expect(switched.reset_reason).toBe("branch_changed");
});

test("reporter recreation reconstructs a native cursor without replay and invalidates old receipts", async () => {
  const f = fixture();
  f.append({ id: "old", type: "message", message: { role: "user", content: "old" } });
  const initial = (await f.read()).result;
  const p = submitParams(initial);
  expect((await f.call("submit", p)).result.status).toBe("accepted_by_reporter");
  const fresh = createConversationAdapter(() => f.ctx);
  const request = (operation: string, params) => fresh({ operation: `conversation_${operation}`, terminal_id: "term-1", request_id: "req-1", params });
  const recovered = (await request("read", { terminal_id: "term-1", expected_context: { conversation_id: "session-1", reset_entry_id: null }, cursor: initial.baseline_cursor, max_bytes: 4096 })).result;
  expect(recovered.records).toEqual([]);
  expect(recovered.reporter_epoch).not.toBe(initial.reporter_epoch);
  expect((await request("input_status", p)).result.status).toBe("unknown");
  expect((await request("submit", p)).error.reason).toBe("reporter_changed");
  expect(f.calls()).toBe(1);
});

test("simultaneous same-ID submissions join a reservation and status lookups never deliver", async () => {
  const f = fixture(), initial = (await f.read()).result;
  const gate = Promise.withResolvers<{ status: string }>();
  let calls = 0;
  f.setSubmit(async () => { calls++; return gate.promise; });
  const p = submitParams(initial), first = f.call("submit", p), duplicate = f.call("submit", p);
  await Promise.resolve();
  expect((await f.call("input_status", p)).result.status).toBe("dispatching");
  expect((await f.call("submit", { ...p, text: "different" })).error.reason).toBe("idempotency_conflict");
  expect(calls).toBe(1);
  gate.resolve({ status: "accepted" });
  const a = await first, b = await duplicate;
  expect(a.result.receipt).toBe(b.result.receipt);
  expect(a.result.status).toBe("accepted_by_reporter");
  expect(a.result).not.toHaveProperty("entry_id");
  expect((await f.call("input_status", p)).result.receipt).toBe(a.result.receipt);
  expect(calls).toBe(1);
});

test("unknown native submission is retained without retry while later deliberate inputs remain possible", async () => {
  const f = fixture(), p = submitParams((await f.read()).result);
  let calls = 0;
  f.setSubmit(async () => { calls++; if (calls === 1) throw new Error("lost admission response"); return { status: "accepted" }; });
  expect((await f.call("submit", p)).error.invocation).toBe("unknown");
  expect((await f.call("input_status", p)).result.status).toBe("unknown");
  expect((await f.call("submit", p)).error.invocation).toBe("unknown");
  expect((await f.call("submit", { ...p, input_id: "input-2" })).result.status).toBe("accepted_by_reporter");
  expect(calls).toBe(2);
});

test("stale context and cancel fence never invoke native work and clear hides earlier history", async () => {
  const f = fixture(), initial = (await f.read()).result, p = submitParams(initial);
  expect((await f.call("abort", { ...p, work_fence: "old-work" })).error.reason).toBe("work_changed");
  expect(f.aborts()).toBe(0);
  f.clear();
  expect((await f.call("submit", p)).error.reason).toBe("binding_mismatch");
  expect((await f.call("abort", { ...p, work_fence: "work-1" })).error.invocation).toBe("not_started");
  expect(f.calls()).toBe(0);
  expect(f.aborts()).toBe(0);
  expect((await f.read()).result.records).toEqual([]);
});

test("final voice policy byte limits reject before invocation and missing native seams fail closed", async () => {
  const f = fixture(), p = submitParams((await f.read()).result);
  const tooLarge = await f.call("submit", { ...p, text: "🙂".repeat(32768) });
  expect(tooLarge.error.reason).toBe("request_too_large");
  expect(tooLarge.error.invocation).toBe("not_started");
  expect(f.calls()).toBe(0);
  const old = createConversationAdapter(() => ({ sessionManager: f.ctx.sessionManager }));
  expect((await old({ operation: "conversation_read", terminal_id: "term-1", request_id: "req-1", params: {} })).error.reason).toBe("structured_unavailable");
});

test("in-progress receipt reservations cannot be evicted under capacity pressure", async () => {
  const f = fixture(), p = submitParams((await f.read()).result);
  const gate = Promise.withResolvers<{ status: string }>();
  let calls = 0;
  f.setSubmit(async () => { calls++; return gate.promise; });
  const requests = Array.from({ length: 256 }, (_, index) => f.call("submit", { ...p, input_id: `input-${index}` }));
  await Promise.resolve();
  const overflow = await f.call("submit", { ...p, input_id: "overflow" });
  expect(overflow.error.reason).toBe("capacity_exhausted");
  expect(overflow.error.invocation).toBe("not_started");
  expect(calls).toBe(256);
  expect((await f.call("input_status", p)).result.status).toBe("dispatching");
  gate.resolve({ status: "accepted" });
  await Promise.all(requests);
  expect((await f.call("submit", { ...p, input_id: "after-settlement" })).result.status).toBe("accepted_by_reporter");
});

test("short multibyte tail fits without false empty-page failure", async () => {
  const f = fixture();
  f.append({ id: "emoji", type: "message", message: { role: "assistant", stopReason: "stop", content: [{ type: "text", text: "🙂" }] } });
  const history = (await f.read()).result;
  expect(history.records[0].text).toBe("🙂");
  expect(history.records[0].entry_end).toBe(true);
  expect(history.has_more).toBe(false);
});

test("branch changes discard unfinished text pages rather than treating a partial answer as new output", async () => {
  const f = fixture();
  f.append({ id: "large", type: "message", message: { role: "assistant", stopReason: "stop", content: [{ type: "text", text: "long answer ".repeat(1200) }] } });
  const first = (await f.read()).result;
  expect(first.has_more).toBe(true);
  expect(first.records[0].entry_end).toBe(false);
  f.setEpoch("context-switched-back");
  const next = (await f.read(first.next_cursor)).result;
  expect(next.mode).toBe("reset");
  expect(next.reset_reason).toBe("branch_changed");
  expect(next.records).toEqual([]);
});

test("context changes during native indexing discard the captured assembly", async () => {
  const f = fixture();
  for (let i = 0; i < 600; i++) f.append({ id: `entry-${i}`, type: "message", message: { role: "user", content: "history" } });
  const pending = f.read();
  // Read callbacks run in a microtask and indexing yields after 512 native entries.
  await Promise.resolve();
  await Promise.resolve();
  f.setEpoch("context-during-index");
  const response = await pending;
  expect(response.error.reason).toBe("context_changed");
  expect(response).not.toHaveProperty("result");
});
