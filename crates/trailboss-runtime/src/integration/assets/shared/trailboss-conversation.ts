// Shared restricted-app conversation adapter for the OMP and Pi reporters.
// The integration installer inlines this module where a reporter imports it,
// so the installed reporter stays one self-contained file. Keep the import
// lines below matched by the importing reporters' own imports.
import { createHash, randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

/** An explicit grant is tied to one live process and conversation, never a tab name. */
export function readRestrictedAppGrant(ctx, directory) {
  if (process.platform !== "linux" || !process.geteuid || ctx?.hasUI !== true) return undefined;
  let fd;
  try {
    const uid = process.geteuid(), parent = fs.lstatSync(directory);
    if (!parent.isDirectory() || parent.isSymbolicLink() || parent.uid !== uid ||
        (parent.mode & 0o777) !== 0o700) return undefined;
    const marker = path.join(directory, `${process.pid}.restricted-app.json`);
    fd = fs.openSync(marker, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW | fs.constants.O_NONBLOCK);
    const stat = fs.fstatSync(fd);
    if (!stat.isFile() || stat.uid !== uid || (stat.mode & 0o777) !== 0o600 ||
        stat.nlink !== 1 || stat.size <= 0 || stat.size > 4096) return undefined;
    const buffer = Buffer.alloc(4097), length = fs.readSync(fd, buffer, 0, buffer.length, 0);
    const current = fs.lstatSync(marker);
    if (length !== stat.size || current.dev !== stat.dev || current.ino !== stat.ino) return undefined;
    const grant = JSON.parse(buffer.subarray(0, length).toString("utf8"));
    const proc = fs.readFileSync("/proc/self/stat", "utf8");
    const start = proc.slice(proc.lastIndexOf(")") + 2).split(/\s+/)[19];
    if (grant?.version !== 1 || grant.pid !== process.pid || grant.proc_start_ticks !== start ||
        typeof grant.terminal_id !== "string" || grant.terminal_id.length === 0 ||
        Buffer.byteLength(grant.terminal_id, "utf8") > 256 || /[\u0000-\u001f\u007f]/.test(grant.terminal_id) ||
        grant.conversation_id !== ctx.sessionManager?.getSessionId?.() ||
        typeof grant.session_path !== "string" || !path.isAbsolute(grant.session_path) ||
        grant.session_path !== ctx.sessionManager?.getSessionFile?.()) return undefined;
    return grant;
  } catch {
    return undefined;
  } finally {
    if (fd !== undefined) fs.closeSync(fd);
  }
}

const CONVERSATION_POLICIES = {
  brief: "[Voice format for this request only: answer with a short actual reply suitable for ten seconds; give detail only if requested. Do not apply this format to unrelated desktop requests.]\n\n",
  expanded: "[Voice format for this request only: more detail is explicitly requested. Do not apply this format to unrelated desktop requests.]\n\n"
};

/** Projects native entries only. Input receipts never identify assistant replies. */
export function createConversationAdapter(getContext, restrictedOptions) {
  const reporterEpoch = randomUUID();
  const receipts = new Map();
  let cachedContext, branch = [], positions = new Map(), tip = null;
  let reads = Promise.resolve();
  let restrictedContext, restrictedEpoch = randomUUID(), restrictedTip, restrictedReset = null;
  let resetCache = new Map(), outstanding = null;
  let metadataTimer, metadataClosed = false;
  const metadataWaiters = new Set();
  function pulseMetadata() {
    for (const wake of metadataWaiters) wake();
  }
  function waitForMetadataPulse(timeout) {
    return new Promise(resolve => {
      const wake = () => {
        if (!metadataWaiters.delete(wake)) return;
        clearTimeout(timer);
        resolve();
      };
      const timer = setTimeout(wake, timeout);
      metadataWaiters.add(wake);
    });
  }
  async function readyMetadata() {
    const ctx = getContext(), grant = restrictedOptions?.readGrant?.(ctx);
    if (!grant || metadataClosed || restrictedContext !== grant.conversation_id ||
        ctx?.sessionManager?.getSessionId?.() !== grant.conversation_id) return null;
    const identity = JSON.stringify([grant.terminal_id, grant.conversation_id, grant.session_path]);
    const epoch = restrictedEpoch, deadline = performance.now() + 750;
    while (!metadataClosed && performance.now() < deadline) {
      // Only the independent timer/lifecycle observer refreshes metadata.
      await waitForMetadataPulse(Math.max(1, deadline - performance.now()));
      const current = getContext(), next = restrictedOptions.readGrant(current);
      if (metadataClosed || !next || restrictedEpoch !== epoch ||
          current?.sessionManager?.getSessionId?.() !== grant.conversation_id ||
          current?.sessionManager?.getSessionFile?.() !== grant.session_path ||
          JSON.stringify([next.terminal_id, next.conversation_id, next.session_path]) !== identity) return null;
      const live = native(true);
      if (live) return live;
    }
    return null;
  }

  function refreshRestricted(ctx) {
    const manager = ctx?.sessionManager, id = manager?.getSessionId?.();
    if (!validId(id) || typeof manager.getLeafId !== "function" || typeof manager.getEntry !== "function") return false;
    if (restrictedContext !== id) {
      restrictedContext = id; restrictedEpoch = randomUUID(); restrictedTip = undefined;
      restrictedReset = null; resetCache = new Map(); outstanding = null;
    }
    const leaf = manager.getLeafId();
    if (!nullableId(leaf)) return false;
    if (leaf === restrictedTip) return true;
    const suffix = [], seen = new Set();
    let cursor = leaf, boundary = null;
    while (cursor !== null && !resetCache.has(cursor)) {
      if (!validId(cursor) || seen.has(cursor)) return false;
      seen.add(cursor);
      const entry = manager.getEntry(cursor);
      if (!entry || !nullableId(entry.parentId)) return false;
      suffix.push(entry.id);
      if (entry.type === "reset_boundary") { boundary = entry.id; break; }
      cursor = entry.parentId;
    }
    if (cursor !== null && resetCache.has(cursor)) boundary = resetCache.get(cursor);
    if ((restrictedTip !== undefined && cursor !== restrictedTip) || boundary !== restrictedReset) {
      restrictedEpoch = randomUUID(); outstanding = null;
    }
    for (const entryId of suffix) resetCache.set(entryId, boundary);
    restrictedTip = leaf; restrictedReset = boundary;
    return true;
  }

  function capabilities(live) {
    if (!live.grant) return ["guarded_submit_v1", "input_status_v1", "guarded_abort_v1", "reply_style_v1"];
    const caps = ["restricted_app_bridge_v1", "single_outstanding_v1", "input_status_v1", "reply_style_v1"];
    if (!outstanding && live.ctx.isIdle() === true && live.ctx.hasPendingMessages() === false &&
        typeof restrictedOptions.sendUserMessage === "function") caps.push("restricted_submit_v1");
    // There is no public input_id-to-work-fence ownership correlation for app cancellation.
    return caps;
  }
  const bytes = value => Buffer.byteLength(JSON.stringify(value), "utf8") + 1;
  const validId = id => typeof id === "string" && id.length > 0 &&
    Buffer.byteLength(id, "utf8") <= 256 && !/[\u0000-\u001f\u007f]/.test(id);
  const nullableId = id => id === null || validId(id);
  const sameContext = (a, b) => a?.conversation_id === b?.conversation_id && a?.reset_entry_id === b?.reset_entry_id;
  const sameBinding = (a, b) => a?.terminal_id === b?.terminal_id && sameContext(a, b);
  const receiptKey = p => JSON.stringify([p.binding.terminal_id, p.binding.conversation_id, p.binding.reset_entry_id, p.input_id]);
  // Compact deterministic binding identity keeps worst-case native IDs inside 2048-byte cursors.
  const bindingKey = binding => createHash("sha256").update(JSON.stringify([
    binding.terminal_id, binding.conversation_id, binding.reset_entry_id
  ])).digest("hex");
  function error(request, reason, invocation = "not_started") {
    return { status: "conversation", reason: "", error: {
      code: reason, message: reason, reason, invocation,
      ...(validId(request.params?.input_id) ? { input_id: request.params.input_id } : {})
    } };
  }
  function native(metadataOnly = false) {
    if (metadataClosed) return null;
    const ctx = getContext(), control = ctx?.conversationControl, manager = ctx?.sessionManager;
    const grant = restrictedOptions?.readGrant?.(ctx);
    if (grant) {
      if (typeof ctx?.isIdle !== "function" || typeof ctx.hasPendingMessages !== "function" ||
          typeof manager?.getSessionId !== "function" || typeof manager.getLeafId !== "function") return null;
      // A metadata read may verify the cached tip, but must never reconstruct history.
      if (metadataOnly ? restrictedContext !== manager.getSessionId() || restrictedTip !== manager.getLeafId() :
          !refreshRestricted(ctx)) return null;
      const idle = ctx.isIdle(), pending = ctx.hasPendingMessages();
      return { ctx, manager, grant, snapshot: {
        conversation_id: restrictedContext, reset_entry_id: restrictedReset, context_epoch: restrictedEpoch,
        work: { state: idle === false ? "working" :
          idle === true && pending === false && !outstanding ? "not_working" : "unknown", fence: null }
      } };
    }
    if (control?.version !== 1 || typeof control.snapshot !== "function" ||
        typeof control.submit !== "function" || typeof control.abort !== "function" ||
        typeof manager?.getLeafId !== "function" || typeof manager?.getEntry !== "function") return null;
    const snapshot = control.snapshot();
    if (!validId(snapshot?.conversation_id) || !nullableId(snapshot.reset_entry_id) ||
        !validId(snapshot.context_epoch) || !["working", "not_working", "unknown"].includes(snapshot.work?.state) ||
        !nullableId(snapshot.work?.fence)) return null;
    return { control, manager, snapshot };
  }
  function encode(cursor) {
    const encoded = Buffer.from(JSON.stringify(cursor), "utf8").toString("base64url");
    if (encoded.length > 2048) throw new Error("invalid_params");
    return encoded;
  }
  function decode(encoded) {
    if (typeof encoded !== "string" || encoded.length > 2048 || !/^[A-Za-z0-9_-]+$/.test(encoded)) throw new Error("invalid_params");
    const c = JSON.parse(Buffer.from(encoded, "base64url").toString("utf8"));
    if (c.v !== 1 || typeof c.b !== "string" || !/^[a-f0-9]{64}$/.test(c.b) ||
        !validId(c.r) || !validId(c.c) || !nullableId(c.t) || !nullableId(c.s) ||
        !Number.isSafeInteger(c.o) || c.o < 0 || !["snapshot", "delta"].includes(c.p)) throw new Error("invalid_params");
    return c;
  }
  function project(entry) {
    const m = entry.message;
    if (entry.type !== "message" || !["user", "assistant"].includes(m?.role) ||
        (m.role === "user" && (m.synthetic === true || (m.attribution !== undefined && m.attribution !== "user")))) return null;
    const text = typeof m.content === "string" ? m.content : Array.isArray(m.content) ?
      m.content.filter(b => b?.type === "text" && typeof b.text === "string").map(b => b.text).join("") : "";
    if (!text) return null;
    return { entry_id: entry.id, parent_id: entry.parentId, role: m.role,
      stop_reason: m.role === "assistant" ? m.stopReason ?? null : null,
      has_tool_calls: m.role === "assistant" ? (Array.isArray(m.content) ?
        m.content.some(b => !["text", "thinking", "redactedThinking"].includes(b?.type)) : null) : false, text };
  }
  async function index(manager, snapshot) {
    const context = JSON.stringify([snapshot.conversation_id, snapshot.reset_entry_id, snapshot.context_epoch]);
    const capturedTip = manager.getLeafId();
    if (!nullableId(capturedTip)) throw new Error("history_unavailable");
    if (cachedContext !== context) { branch = []; positions = new Map(); tip = null; cachedContext = context; }
    if (capturedTip === tip) return capturedTip;
    const suffix = [], seen = new Set();
    let id = capturedTip;
    while (id !== tip && id !== snapshot.reset_entry_id && id !== null) {
      if (!validId(id) || seen.has(id)) throw new Error("history_unavailable");
      seen.add(id);
      const entry = manager.getEntry(id);
      if (!entry || !nullableId(entry.parentId)) throw new Error("history_unavailable");
      suffix.push(entry); id = entry.parentId;
      if (suffix.length % 512 === 0) {
        const deferred = Promise.withResolvers();
        setImmediate(deferred.resolve);
        await deferred.promise;
      }
    }
    if (tip !== null && id !== tip) throw new Error("branch_changed");
    for (const entry of suffix.reverse()) { positions.set(entry.id, branch.length); branch.push(entry); }
    tip = capturedTip;
    return capturedTip;
  }
  async function read(request, live) {
    const p = request.params, budget = p.max_bytes ?? 65536, snapshot = live.snapshot;
    if (!Number.isSafeInteger(budget) || (budget !== 0 && (budget < 4096 || budget > 65536)) ||
        p.terminal_id !== request.terminal_id || (p.cursor != null && p.expected_context == null)) return error(request, "invalid_params");
    const binding = { terminal_id: request.terminal_id, conversation_id: snapshot.conversation_id, reset_entry_id: snapshot.reset_entry_id };
    if (p.expected_context != null && !sameContext(p.expected_context, binding)) return error(request, "binding_mismatch");
    // Zero is a metadata request, not a history page with a zero-size budget.
    if (budget === 0) return { status: "conversation", reason: "", result: {
      contract_version: 1, binding, reporter_epoch: reporterEpoch, context_epoch: snapshot.context_epoch,
      work: snapshot.work, capabilities: capabilities(live),
      mode: "unchanged", reset_reason: null, snapshot_tip_id: null, records: [],
      next_cursor: null, baseline_cursor: null, has_more: false
    } };
    const capturedTip = live.manager.getLeafId();
    let cursor = p.cursor == null ? null : decode(p.cursor);
    const result = { contract_version: 1, binding, reporter_epoch: reporterEpoch, context_epoch: snapshot.context_epoch,
      work: snapshot.work, capabilities: capabilities(live),
      mode: "delta", reset_reason: null, snapshot_tip_id: capturedTip, records: [],
      next_cursor: null, baseline_cursor: null, has_more: false };
    const fits = value => Math.max(bytes({ status: "conversation", reason: "", result: value }),
      bytes({ id: request.request_id, result: { type: "agent_conversation", result: value } })) <= budget;
    const reset = reason => {
      const resetResult = { ...result, mode: "reset", reset_reason: reason };
      return fits(resetResult) ? { status: "conversation", reason: "", result: resetResult } : error(request, "page_budget_too_small");
    };
    try { await index(live.manager, snapshot); }
    catch (cause) {
      cachedContext = undefined;
      return reset(cause?.message === "branch_changed" ? "branch_changed" : "history_unavailable");
    }
    const now = native();
    if (!now || !sameContext(now.snapshot, snapshot) || now.snapshot.context_epoch !== snapshot.context_epoch) return error(request, "context_changed");
    if (cursor && (cursor.b !== bindingKey(binding) || (cursor.r === reporterEpoch && cursor.c !== snapshot.context_epoch))) return reset("branch_changed");
    const isPosition = id => id === null || id === binding.reset_entry_id || positions.has(id);
    const position = id => id === null || id === binding.reset_entry_id ? -1 : positions.get(id);
    if (cursor && (!isPosition(cursor.t) || !isPosition(cursor.s))) return reset("history_unavailable");
    if (cursor && position(cursor.s) > position(cursor.t)) return reset("branch_changed");
    if (cursor?.o > 0 && position(cursor.s) < 0) return reset("history_unavailable");
    const fresh = (s, p, t = capturedTip, o = 0) => ({ v: 1, b: bindingKey(binding), r: reporterEpoch, c: snapshot.context_epoch, t, s, o, p });
    let start;
    if (!cursor) {
      result.mode = "snapshot"; result.baseline_cursor = encode(fresh(capturedTip, "delta"));
      start = Math.max(0, branch.length - 64);
      cursor = fresh(start > 0 ? branch[start - 1].id : binding.reset_entry_id, "snapshot");
    } else {
      result.mode = cursor.p; cursor = { ...cursor, r: reporterEpoch, c: snapshot.context_epoch };
      if (cursor.s === cursor.t && cursor.o === 0) { cursor = fresh(cursor.s, "delta"); result.mode = "delta"; }
      start = position(cursor.s) + (cursor.o > 0 ? 0 : 1);
    }
    result.snapshot_tip_id = cursor.t;
    const end = position(cursor.t);
    for (let i = start, scanned = 0; i <= end && scanned < 4096; i++, scanned++) {
      const entry = branch[i], visible = project(entry);
      if (!visible) {
        const next = { ...cursor, s: entry.id, o: 0, p: i === end ? "delta" : cursor.p };
        if (!fits({ ...result, next_cursor: encode(next), has_more: i < end })) {
          if (result.records.length === 0) return error(request, "page_budget_too_small");
          break;
        }
        cursor = next;
        continue;
      }
      const text = Buffer.from(visible.text, "utf8"), offset = cursor.o;
      if (offset >= text.length || (offset > 0 && (text[offset] & 0xc0) === 0x80)) return reset("history_unavailable");
      const record = { ...visible, text_total_bytes: text.length, text_offset_bytes: offset, text: "", entry_end: false };
      let low = 0, high = text.length - offset, best = 0;
      while (low <= high) {
        const probe = Math.floor((low + high) / 2);
        let length = probe;
        while (length > 0 && offset + length < text.length && (text[offset + length] & 0xc0) === 0x80) length--;
        if (length === 0) { low = probe + 1; continue; }
        const complete = offset + length === text.length;
        const next = { ...cursor, s: entry.id, o: complete ? 0 : offset + length, p: complete && i === end ? "delta" : cursor.p };
        const candidate = { ...record, text: text.subarray(offset, offset + length).toString("utf8"), entry_end: complete };
        if (length > 0 && fits({ ...result, records: [...result.records, candidate], next_cursor: encode(next), has_more: !complete || i < end })) {
          best = length; low = probe + 1;
        } else high = probe - 1;
      }
      if (best === 0) { if (result.records.length === 0) return error(request, "page_budget_too_small"); break; }
      record.text = text.subarray(offset, offset + best).toString("utf8"); record.entry_end = offset + best === text.length;
      result.records.push(record); cursor.s = entry.id; cursor.o = record.entry_end ? 0 : offset + best;
      if (!record.entry_end) break;
    }
    result.has_more = cursor.o > 0 || position(cursor.s) < end;
    if (!result.has_more) cursor.p = "delta";
    result.next_cursor = encode(cursor);
    if (p.cursor && result.records.length === 0 && !result.has_more && cursor.s === capturedTip &&
        decode(p.cursor).s === capturedTip && decode(p.cursor).o === 0) result.mode = "unchanged";
    if (!fits(result)) return error(request, "page_budget_too_small");
    const after = native();
    if (!after || !sameContext(after.snapshot, snapshot) || after.snapshot.context_epoch !== snapshot.context_epoch) return error(request, "context_changed");
    return { status: "conversation", reason: "", result };
  }
  async function dispatch(request) {
    const p = request.params;
    if (!p || !validId(request.terminal_id) || !validId(request.request_id)) return error(request, "invalid_params");
    if (request.operation !== "conversation_read" && (!p.binding || !validId(p.binding.terminal_id) || !validId(p.binding.conversation_id) ||
        !nullableId(p.binding.reset_entry_id) || !validId(p.reporter_epoch))) return error(request, "invalid_params");
    const key = request.operation === "conversation_read" ? null : receiptKey(p);
    if (request.operation === "conversation_input_status") {
      if (!validId(p.input_id)) return error(request, "invalid_params");
      const slot = p.reporter_epoch === reporterEpoch ? receipts.get(key) : undefined;
      return { status: "conversation", reason: "", result: slot ? slot.result : {
        input_id: p.input_id, binding: p.binding, reporter_epoch: reporterEpoch, status: "unknown",
        reason: p.reporter_epoch === reporterEpoch ? "receipt_unavailable" : "reporter_changed" } };
    }
    let live;
    try { live = native(request.operation === "conversation_capabilities" ||
      (request.operation === "conversation_read" && p.max_bytes === 0)); } catch { return error(request, "structured_unavailable"); }
    if (!live && request.operation === "conversation_read" && p.max_bytes === 0) live = await readyMetadata();
    if (!live) return error(request, "structured_unavailable");
    if (live.grant && live.grant.terminal_id !== request.terminal_id) return error(request, "binding_mismatch");
    const binding = { terminal_id: request.terminal_id, conversation_id: live.snapshot.conversation_id, reset_entry_id: live.snapshot.reset_entry_id };
    if (request.operation === "conversation_read") return read(request, live);
    if (request.operation === "conversation_capabilities") return { status: "conversation", reason: "", result: {
      contract_version: 1, binding, reporter_epoch: reporterEpoch, context_epoch: live.snapshot.context_epoch,
      capabilities: capabilities(live)
    } };
    if (!sameBinding(p.binding, binding)) return error(request, "binding_mismatch");
    if (p.reporter_epoch !== reporterEpoch) return error(request, "reporter_changed");
    if (!validId(p.context_epoch) || p.context_epoch !== live.snapshot.context_epoch) return error(request, "context_changed");
    const expected_context = { conversation_id: binding.conversation_id, reset_entry_id: binding.reset_entry_id };
    if (request.operation === "conversation_abort") {
      if (live.grant) return error(request, "ownership_unknown");
      if (!validId(p.work_fence)) return error(request, "invalid_params");
      try {
        const outcome = await live.control.abort({ expected_context, context_epoch: p.context_epoch, work_fence: p.work_fence });
        if (outcome?.status === "rejected") return error(request, outcome.reason ?? "work_changed", outcome.invocation === "not_started" ? "not_started" : "unknown");
        if (!["acknowledged", "not_working"].includes(outcome?.status)) return error(request, "abort_failed", "unknown");
        return { status: "conversation", reason: "", result: { binding, reporter_epoch: reporterEpoch, context_epoch: p.context_epoch, status: outcome.status } };
      } catch { return error(request, "abort_failed", "unknown"); }
    }
    if (request.operation !== "conversation_submit") return error(request, "structured_unavailable");
    if (!validId(p.input_id) || typeof p.text !== "string" || !p.text.trim() ||
        !Object.hasOwn(CONVERSATION_POLICIES, p.reply_style)) return error(request, "invalid_params");
    const policy = live.grant ? {
      brief: "[Reply format for this request only: be concise; preserve the app's requested result schema.]\n\n",
      expanded: "[Reply format for this request only: provide the detail required by the app's result schema.]\n\n"
    } : CONVERSATION_POLICIES;
    const text = policy[p.reply_style] + p.text;
    if (Buffer.byteLength(text, "utf8") > 128 * 1024 || bytes({ ...request, params: { ...p, text } }) > 256 * 1024) return error(request, "request_too_large");
    const digest = createHash("sha256").update(JSON.stringify([p.text, p.reply_style])).digest("hex"), previous = receipts.get(key);
    if (previous) return previous.digest === digest ? previous.promise : error(request, "idempotency_conflict");
    if (live.grant) {
      if (outstanding) return error(request, "instruction_outstanding");
      if (live.ctx.isIdle() !== true || live.ctx.hasPendingMessages() !== false ||
          typeof restrictedOptions.sendUserMessage !== "function") return error(request, "agent_busy");
    }
    if (receipts.size >= 256) {
      const oldest = [...receipts].find(([, slot]) => slot.result.status !== "dispatching");
      if (!oldest) return error(request, "capacity_exhausted");
      receipts.delete(oldest[0]);
    }
    const slot = { digest, settingUp: true, started: false, completed: false,
      result: { input_id: p.input_id, binding, reporter_epoch: reporterEpoch, context_epoch: p.context_epoch, status: "dispatching" }, promise: null };
    // Reserve before native invocation/await; concurrent duplicates join the reservation.
    receipts.set(key, slot);
    if (live.grant) outstanding = slot;
    slot.promise = Promise.resolve().then(async () => {
      try {
        let outcome;
        if (live.grant) {
          let fresh;
          try { fresh = native(); } catch {}
          const reason = !fresh?.grant ? "structured_unavailable" :
            fresh.grant.terminal_id !== request.terminal_id || !sameContext(fresh.snapshot, binding) ? "binding_mismatch" :
            fresh.snapshot.context_epoch !== p.context_epoch ? "context_changed" :
            outstanding !== slot ? "instruction_outstanding" :
            fresh.ctx.isIdle() !== true || fresh.ctx.hasPendingMessages() !== false ? "agent_busy" : null;
          if (reason) outcome = { status: "rejected", reason, invocation: "not_started" };
          else {
            // No await between the actual idle/queue admission check and public send.
            await restrictedOptions.sendUserMessage(text);
            outcome = { status: "accepted" };
          }
        } else outcome = await live.control.submit({ expected_context, context_epoch: p.context_epoch, text });
        if (outcome?.status === "accepted") {
          slot.result = { ...slot.result, status: "accepted_by_reporter", receipt: randomUUID(), reply_style: p.reply_style };
          return { status: "conversation", reason: "", result: slot.result };
        }
        const invocation = outcome?.invocation === "not_started" ? "not_started" : "unknown", reason = outcome?.reason ?? "submission_failed";
        slot.result = { ...slot.result, status: invocation === "not_started" ? "rejected_before_dispatch" : "unknown", reason };
        return error(request, reason, invocation);
      } catch {
        slot.result = { ...slot.result, status: "unknown", reason: "submission_failed" };
        return error(request, "submission_failed", "unknown");
      } finally {
        slot.settingUp = false;
        if (outstanding === slot && (slot.completed || slot.result.status === "rejected_before_dispatch")) outstanding = null;
      }
    });
    return slot.promise;
  }
  const adapter = request => {
    const run = () => dispatch(request).catch(cause => error(request,
      request.operation === "conversation_read" && ["invalid_params", "history_unavailable", "branch_changed"].includes(cause?.message) ? cause.message : "structured_unavailable",
      request.operation === "conversation_read" ? "not_started" : "unknown"));
    if (request.operation !== "conversation_read") return run();
    const result = reads.then(run, run); reads = result.then(() => {}, () => {}); return result;
  };
  adapter.observe = (name, event, ctx) => {
    if (metadataClosed || !restrictedOptions || ctx?.hasUI !== true) return;
    if (!metadataTimer) {
      // Public hooks can precede journal appends, including during busy tools.
      // Refresh opted-in processes independently of metadata read requests.
      metadataTimer = setInterval(() => {
        try {
          const current = getContext();
          if (current?.hasUI === true && restrictedOptions.readGrant(current)) refreshRestricted(current);
        } catch {}
        pulseMetadata();
      }, 250);
      metadataTimer.unref?.();
    }
    // Only a started app turn holds the conversation. A send that never starts a turn (no model,
    // usage preflight, cost cap, Esc during setup) must not refuse /new, /resume, /fork or /tree;
    // the transition below then releases the unstarted reservation.
    if (name.startsWith("session_before_") && outstanding?.started && restrictedOptions.readGrant(ctx)) return { cancel: true };
    if (["session_switch", "session_branch", "session_tree"].includes(name)) {
      restrictedEpoch = randomUUID(); restrictedTip = undefined; outstanding = null;
    }
    try { if (!refreshRestricted(ctx)) return; } catch { return; }
    finally { pulseMetadata(); }
    if (name === "agent_start" && outstanding) outstanding.started = true;
    // OMP reports a final run on agent_end; Pi reports it on agent_settled.
    if ((name === "agent_end" && event?.willContinue !== true) || name === "agent_settled") {
      const slot = outstanding, epoch = restrictedEpoch;
      const settle = () => {
        if (!slot || outstanding !== slot || restrictedEpoch !== epoch || !slot.started ||
            ctx.isIdle() !== true || ctx.hasPendingMessages() !== false) return;
        if (slot.settingUp) slot.completed = true;
        else outstanding = null;
      };
      settle();
      if (outstanding === slot) setImmediate(settle);
    }
  };
  adapter.close = () => {
    metadataClosed = true;
    clearInterval(metadataTimer);
    metadataTimer = undefined;
    pulseMetadata();
  };
  return adapter;
}
