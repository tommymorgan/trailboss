// installed by trailboss
// managed by trailboss; reinstalling or updating the integration overwrites this file.
// add custom hooks/plugins beside this file instead of editing it.
// TRAILBOSS_INTEGRATION_ID=omp
// TRAILBOSS_INTEGRATION_VERSION=17
// @ts-nocheck

import net, { type Server, type Socket } from "node:net";
import fs from "node:fs";
import path from "node:path";

const HERDR_ENV = process.env.HERDR_ENV;
const socketPath = process.env.HERDR_SOCKET_PATH;
const socketEndpoint =
  process.platform === "win32" && socketPath ? `\\\\.\\pipe\\${socketPath}` : socketPath;
const paneId = process.env.HERDR_PANE_ID;
const source = "herdr:omp";

function procEnvironValue(environ: string, name: string): string | undefined {
  for (const entry of environ.split("\0")) {
    if (entry.startsWith(`${name}=`)) {
      return entry.slice(name.length + 1);
    }
  }
  return undefined;
}

function readProcText(pid: number, leaf: string): string | undefined {
  try {
    return fs.readFileSync(`/proc/${pid}/${leaf}`, "utf8");
  } catch {
    return undefined;
  }
}

function procParentPid(pid: number): number | undefined {
  const status = readProcText(pid, "status");
  const match = status ? /^PPid:\s+(\d+)$/m.exec(status) : null;
  return match ? Number(match[1]) : undefined;
}

// Linux-only, once at module load: a nested `omp` can also be spawned directly
// by the pane's root agent, without a marked shell in between. A process whose
// ancestor chain still inside this pane's context contains an `omp` process is
// such a nested child. The pane context is the boundary: the Core overwrites
// HERDR_SOCKET_PATH and HERDR_PANE_ID when spawning a pane, so the first
// ancestor whose initial environment no longer matches this pane's values is
// outside the pane (the Core or beyond); the walk stops before classifying or
// traversing it. Unverifiable ancestry (unreadable hop, pid<=1, depth cap)
// fails open to OMPCODE-only behavior, so a genuine root keeps reporting.
function ompAncestorWithinPaneContext(): boolean {
  if (process.platform !== "linux" || !socketPath || !paneId) {
    return false;
  }
  let pid = process.ppid;
  for (let hop = 0; pid > 1 && hop < 32; hop += 1) {
    const environ = readProcText(pid, "environ");
    if (environ === undefined) {
      return false;
    }
    if (
      procEnvironValue(environ, "HERDR_SOCKET_PATH") !== socketPath ||
      procEnvironValue(environ, "HERDR_PANE_ID") !== paneId
    ) {
      return false;
    }
    let name = readProcText(pid, "comm")?.trim();
    if (!name) {
      try {
        name = path.basename(fs.readlinkSync(`/proc/${pid}/exe`));
      } catch {
        name = undefined;
      }
    }
    if (name === "omp") {
      return true;
    }
    const parent = procParentPid(pid);
    if (parent === undefined || parent <= 1) {
      return false;
    }
    pid = parent;
  }
  return false;
}

// OMP marks every shell it spawns with OMPCODE=1. A nested `omp` launched from
// a parent session's shell inherits it, so that process is not the pane's root
// agent and must not report its short-lived session over the parent's.
const nestedOmpSession = process.env.OMPCODE === "1" || ompAncestorWithinPaneContext();

function enabled() {
  return HERDR_ENV === "1" && !!socketPath && !!paneId && !nestedOmpSession;
}

let requestQueue = Promise.resolve();

function sendRequestAttempt(request: unknown, timeoutMs: number): Promise<boolean> {
  if (!enabled()) {
    return Promise.resolve(true);
  }

  return new Promise((resolve) => {
    let done = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const finish = (delivered: boolean) => {
      if (done) return;
      done = true;
      if (timeout) {
        clearTimeout(timeout);
      }
      socket.destroy();
      resolve(delivered);
    };

    const socket = net.createConnection(socketEndpoint!);
    socket.on("error", () => finish(false));
    socket.on("connect", () => socket.write(`${JSON.stringify(request)}\n`));
    socket.on("data", () => finish(true));
    socket.on("end", () => finish(false));
    timeout = setTimeout(() => finish(false), timeoutMs);
    timeout.unref?.();
  });
}

async function sendRequestNow(request: unknown): Promise<void> {
  if (await sendRequestAttempt(request, 500)) {
    return;
  }
  await sendRequestAttempt(request, 1500);
}

function sendRequest(request: unknown): Promise<void> {
  requestQueue = requestQueue.then(
    () => sendRequestNow(request),
    () => sendRequestNow(request),
  );
  return requestQueue;
}

type AgentState = "working" | "waiting" | "blocked" | "idle";

type QueuedState = {
  state: AgentState;
  message?: string;
  seq: number;
};

const idleDebounceMs = parseDurationEnv("TRAILBOSS_OMP_IDLE_DEBOUNCE_MS", 250);
const retryGraceMs = parseDurationEnv("TRAILBOSS_OMP_RETRY_GRACE_MS", 2500);
const retryableErrorPattern =
  /overloaded|provider.?returned.?error|rate.?limit|too many requests|429|500|502|503|504|service.?unavailable|server.?error|internal.?error|network.?error|connection.?error|connection.?refused|connection.?lost|websocket.?closed|websocket.?error|other side closed|fetch failed|upstream.?connect|reset before headers|socket hang up|ended without|http2 request did not get a response|timed? out|timeout|terminated|retry delay/i;
let reportSeq = Date.now() * 1000;
let currentAgentSessionId: string | undefined;
let currentAgentSessionPath: string | undefined;

function nextReportSeq(): number {
  reportSeq += 1;
  return reportSeq;
}

export function isAbsoluteSessionPath(file: unknown): file is string {
  return (
    typeof file === "string" &&
    (path.posix.isAbsolute(file) || path.win32.isAbsolute(file))
  );
}

function updateSessionRef(ctx: any): void {
  try {
    const file = ctx?.sessionManager?.getSessionFile?.();
    currentAgentSessionPath = isAbsoluteSessionPath(file) ? file : undefined;
  } catch {
    currentAgentSessionPath = undefined;
  }

  try {
    const id = ctx?.sessionManager?.getSessionId?.();
    currentAgentSessionId = typeof id === "string" && id.length > 0 ? id : undefined;
  } catch {
    currentAgentSessionId = undefined;
  }
}

function withSessionRef(params: Record<string, unknown>): Record<string, unknown> {
  if (currentAgentSessionPath) {
    return { ...params, agent_session_path: currentAgentSessionPath };
  }
  if (currentAgentSessionId) {
    return { ...params, agent_session_id: currentAgentSessionId };
  }
  return params;
}

function parseDurationEnv(name: string, fallback: number): number {
  const raw = process.env[name];
  if (!raw) {
    return fallback;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return fallback;
  }
  return parsed;
}

function currentSessionRef(): Record<string, unknown> | undefined {
  if (currentAgentSessionPath) {
    return { agent_session_path: currentAgentSessionPath };
  }
  if (currentAgentSessionId) {
    return { agent_session_id: currentAgentSessionId };
  }
  return undefined;
}

function reportSession(sessionStartSource = "startup"): Promise<void> {
  const sessionRef = currentSessionRef();
  if (!sessionRef) {
    return Promise.resolve();
  }

  return sendRequest({
    id: `${source}:session:${Date.now()}:${Math.random().toString(36).slice(2)}`,
    method: "pane.report_agent_session",
    params: {
      pane_id: paneId,
      source,
      agent: "omp",
      seq: nextReportSeq(),
      session_start_source: sessionStartSource,
      ...sessionRef,
    },
  });
}

function sendState(state: AgentState, message?: string, seq = nextReportSeq()): Promise<void> {
  return sendRequest({
    id: `${source}:${Date.now()}:${Math.random().toString(36).slice(2)}`,
    method: "pane.report_agent",
    params: withSessionRef({
      pane_id: paneId,
      source,
      agent: "omp",
      state,
      message,
      seq,
    }),
  });
}

let sendInFlight = false;
let queuedState: QueuedState | undefined;

function queueState(state: AgentState, message?: string): void {
  queuedState = { state, message, seq: nextReportSeq() };
  if (!sendInFlight) {
    void drainStateQueue();
  }
}

async function drainStateQueue(): Promise<void> {
  if (sendInFlight) {
    return;
  }

  sendInFlight = true;
  try {
    while (queuedState) {
      const next = queuedState;
      queuedState = undefined;
      await sendState(next.state, next.message, next.seq);
    }
  } finally {
    sendInFlight = false;
    if (queuedState) {
      void drainStateQueue();
    }
  }
}

function lastAssistantMessage(messages: unknown[]): any | undefined {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i] as any;
    if (message?.role === "assistant") {
      return message;
    }
  }
  return undefined;
}

function retryableErrorMessage(event: any): string | undefined {
  const messages = Array.isArray(event?.messages) ? event.messages : [];
  const assistant = lastAssistantMessage(messages);
  if (assistant?.stopReason !== "error") {
    return undefined;
  }

  const errorMessage = String(assistant.errorMessage ?? "");
  if (!retryableErrorPattern.test(errorMessage)) {
    return undefined;
  }
  return errorMessage || "retryable provider error";
}

function askWaitingMessage(args: any): string {
  const questions = Array.isArray(args?.questions) ? args.questions : [];
  const firstQuestion = questions.find((question: any) => typeof question?.question === "string");
  if (firstQuestion?.question) {
    return firstQuestion.question;
  }
  return "waiting for user input";
}

type RestartContext = {
  hasUI?: boolean;
  sessionManager?: {
    getSessionFile?: () => unknown;
    getSessionId?: () => unknown;
  };
  isIdle?: () => boolean;
  hasPendingMessages?: () => boolean;
  ui?: { getEditorText?: () => string };
  getAsyncJobSnapshot?: () => {
    running: unknown[];
    recent: unknown[];
    delivery: { queued: number; delivering: boolean };
  } | null;
  shutdown?: () => void;
};

export default function (pi) {
  if (!enabled()) {
    return;
  }

  let agentActive = false;
  let backgroundAwait = false;
  let retryHoldActive = false;
  let failureBlocked = false;
  let failureMessage: string | undefined;
  let explicitBlockedCount = 0;
  let explicitBlockedMessage: string | undefined;
  let waitingCount = 0;
  let waitingMessage: string | undefined;
  let lastState: AgentState | undefined;
  let lastMessage: string | undefined;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let rootSession = false;
  let currentContext: RestartContext | undefined;
  let controlServer: Server | undefined;
  let ownedSocket: { path: string; dev: number; ino: number } | undefined;
  let restartAccepted = false;
  let controlClosed = false;
  const controlClients = new Set<Socket>();
  const acceptedClients = new Set<Socket>();

  function captureContext(ctx: RestartContext | undefined) {
    currentContext = ctx;
    updateSessionRef(ctx);
  }

  function removeOwnedSocket() {
    if (!ownedSocket) return;
    try {
      const stat = fs.lstatSync(ownedSocket.path);
      if (stat.isSocket() && stat.dev === ownedSocket.dev && stat.ino === ownedSocket.ino) {
        fs.unlinkSync(ownedSocket.path);
      }
    } catch {
      // A missing or replaced endpoint is no longer ours to remove.
    }
    ownedSocket = undefined;
  }

  function closeControl() {
    controlClosed = true;
    controlServer?.close(() => {});
    removeOwnedSocket();
    for (const client of controlClients) {
      // Graceful shutdown can emit session_shutdown before the reply is flushed.
      if (!acceptedClients.has(client)) client.destroy();
    }
    currentContext = undefined;
  }

  function restartSkipReason(request: unknown): string | undefined {
    if (!request || typeof request !== "object" || Array.isArray(request)) return "invalid-request";
    if (!("operation" in request) || !("pid" in request) ||
        !("pane_id" in request) || !("session_path" in request) ||
        request.operation !== "restart-if-idle" || !Number.isSafeInteger(request.pid) ||
        typeof request.pane_id !== "string" || typeof request.session_path !== "string" ||
        !path.posix.isAbsolute(request.session_path)) return "invalid-request";
    if (request.pid !== process.pid) return "pid-mismatch";
    if (request.pane_id !== paneId) return "pane-mismatch";
    if (restartAccepted) return "restart-already-accepted";
    const ctx = currentContext;
    if (controlClosed || !rootSession || ctx?.hasUI !== true) return "interactive-context-unavailable";
    // Re-read the live manager: cached report identity is not admission authority.
    updateSessionRef(ctx);
    if (!currentAgentSessionPath || request.session_path !== currentAgentSessionPath) return "session-mismatch";
    if (desiredState().state !== "idle") return "reporter-not-idle";
    if (typeof ctx.isIdle !== "function" || typeof ctx.hasPendingMessages !== "function" ||
        typeof ctx.ui?.getEditorText !== "function" || typeof ctx.getAsyncJobSnapshot !== "function" ||
        typeof ctx.shutdown !== "function") return "runtime-capability-unavailable";
    try {
      if (ctx.isIdle() !== true) return "runtime-not-idle";
      if (ctx.hasPendingMessages() !== false) return "pending-messages";
      if (ctx.ui.getEditorText() !== "") return "editor-not-empty";
      const jobs = ctx.getAsyncJobSnapshot();
      if (jobs !== null) {
        if (!jobs || !Array.isArray(jobs.running) || !Array.isArray(jobs.recent) ||
            !Number.isSafeInteger(jobs.delivery?.queued) || jobs.delivery.queued < 0 ||
            typeof jobs.delivery.delivering !== "boolean") return "async-state-unavailable";
        if (jobs.running.length > 0) return "async-jobs-running";
        if (jobs.delivery.queued > 0 || jobs.delivery.delivering) return "async-delivery-pending";
      }
    } catch {
      return "runtime-state-unavailable";
    }
    return undefined;
  }

  // Structured chat-submit and turn-abort admission (v13). Both share the
  // identity checks with restart admission but allow working agents.
  const chatTurns = new Map<string, string>();
  const CHAT_FRAME_LIMIT = 256 * 1024;
  const CHAT_TEXT_LIMIT = 128 * 1024;
  const CHAT_KEY_LIMIT = 256;
  const ATTACHMENT_MAX_BYTES = 32 * 1024 * 1024;
  const ATTACHMENTS_MAX_TOTAL_BYTES = 64 * 1024 * 1024;
  const CHAT_TURNS_MAX = 256;


  function baseAdmissionSkipReason(request: unknown): string | undefined {
    if (!request || typeof request !== "object" || Array.isArray(request)) return "invalid-request";
    if (!("operation" in request) || !("pid" in request) ||
        !("pane_id" in request) || !("session_path" in request) ||
        !Number.isSafeInteger(request.pid) ||
        typeof request.pane_id !== "string" || typeof request.session_path !== "string" ||
        !path.posix.isAbsolute(request.session_path)) return "invalid-request";
    if (request.pid !== process.pid) return "pid-mismatch";
    if (request.pane_id !== paneId) return "pane-mismatch";
    if (controlClosed || !rootSession) return "interactive-context-unavailable";
    if (!currentAgentSessionPath || request.session_path !== currentAgentSessionPath) {
      return "session-mismatch";
    }
    return undefined;
  }

  function chatSubmitSkipReason(request: Record<string, unknown>): string | undefined {
    const base = baseAdmissionSkipReason(request);
    if (base) return base;
    if (request.operation !== "chat_submit") return "invalid-request";
    const text = request.text;
    const key = request.idempotency_key;
    if (typeof text !== "string" || text.length === 0 || text.length > CHAT_TEXT_LIMIT) {
      return "invalid-request";
    }
    if (typeof key !== "string" || key.length === 0 || key.length > CHAT_KEY_LIMIT ||
        /[\s\u0000-\u001f\u007f]/.test(key)) {
      return "invalid-request";
    }
    const attachments = request.attachments;
    if (attachments !== undefined) {
      if (!Array.isArray(attachments) || attachments.length > 8) return "invalid-request";
      let total = 0;
      for (const attachment of attachments) {
        if (!attachment || typeof attachment !== "object") return "invalid-request";
        if (typeof attachment.path !== "string" || !attachment.path.startsWith("/") ||
            /[\u0000-\u001f\u007f]/.test(attachment.path)) return "invalid-request";
        if (attachment.kind !== "image" && attachment.kind !== "file") return "invalid-request";
        if (attachment.name !== undefined &&
            (typeof attachment.name !== "string" || attachment.name.length > 256)) {
          return "invalid-request";
        }
        let stat;
        try {
          stat = fs.statSync(attachment.path);
        } catch {
          return "attachment-missing";
        }
        if (!stat.isFile()) return "invalid-request";
        if (stat.size > ATTACHMENT_MAX_BYTES) return "attachment-too-large";
        total += stat.size;
        if (total > ATTACHMENTS_MAX_TOTAL_BYTES) return "attachment-too-large";
        if (attachment.kind === "image" && !imageMimeType(attachment.path)) {
          return "unsupported-image";
        }
      }
    }
    if (typeof pi?.sendUserMessage !== "function") return "structured-capability-unavailable";
    return undefined;
  }

  function imageMimeType(path) {
    const extension = path.slice(path.lastIndexOf(".")).toLowerCase();
    return ({
      ".png": "image/png",
      ".jpg": "image/jpeg",
      ".jpeg": "image/jpeg",
      ".gif": "image/gif",
      ".webp": "image/webp",
    })[extension];
  }

  function buildChatContent(request) {
    let text = request.text;
    const images = [];
    for (const attachment of request.attachments || []) {
      if (attachment.kind === "file") {
        const label = attachment.name || attachment.path.slice(attachment.path.lastIndexOf("/") + 1);
        text += `\n[attached file: ${label}: ${attachment.path}]`;
      } else {
        images.push({
          type: "image",
          data: fs.readFileSync(attachment.path).toString("base64"),
          mimeType: imageMimeType(attachment.path),
        });
      }
    }
    if (images.length === 0) return text;
    return [{ type: "text", text }, ...images];
  }

  function abortSkipReason(request: Record<string, unknown>): string | undefined {
    const base = baseAdmissionSkipReason(request);
    if (base) return base;
    if (request.operation !== "abort") return "invalid-request";
    if (typeof currentContext?.abort !== "function") return "structured-capability-unavailable";
    if (desiredState().state !== "working") return "agent-not-working";
    return undefined;
  }

  function shutdownSkipReason(request: Record<string, unknown>): string | undefined {
    const base = baseAdmissionSkipReason(request);
    if (base) return base;
    if (request.operation !== "shutdown") return "invalid-request";
    const ctx = currentContext;
    if (typeof ctx?.shutdown !== "function") return "structured-capability-unavailable";
    return undefined;
  }

  function startControl() {
    if (controlServer || controlClosed || process.platform !== "linux" || !process.geteuid) return;
    const uid = process.geteuid();
    const directory = `/tmp/trailboss-omp-control-${uid}`;
    const endpoint = `${directory}/${process.pid}.sock`;
    try {
      try {
        fs.mkdirSync(directory, { mode: 0o700 });
      } catch (error) {
        if (error?.code !== "EEXIST") return;
      }
      const directoryStat = fs.lstatSync(directory);
      if (!directoryStat.isDirectory() || directoryStat.isSymbolicLink() ||
          directoryStat.uid !== uid || (directoryStat.mode & 0o777) !== 0o700) return;
      // Never unlink an existing endpoint, even if it appears stale.
      try {
        fs.lstatSync(endpoint);
        return;
      } catch (error) {
        if (error?.code !== "ENOENT") return;
      }
      controlServer = net.createServer((client) => {
        controlClients.add(client);
        client.unref();
        client.setTimeout(1000, () => client.destroy());
        const deadline = setTimeout(() => client.destroy(), 1000);
        deadline.unref();
        client.on("error", () => client.destroy());
        client.on("close", () => {
          clearTimeout(deadline);
          controlClients.delete(client);
          acceptedClients.delete(client);
        });
        let input = Buffer.alloc(0);
        let replied = false;
        client.on("data", (chunk) => {
          if (replied) return;
          const tooLarge = input.length + chunk.length > CHAT_FRAME_LIMIT;
          if (!tooLarge) input = Buffer.concat([input, chunk]);
          const newline = input.indexOf(10);
          let reason: string | undefined;
          let outcome: { status: string; reason: string; turn_id?: string } | undefined;
          if (tooLarge) reason = "request-too-large";
          else if (newline < 0) return;
          else if (newline !== input.length - 1) reason = "invalid-frame";
          else {
            try {
              const parsed = JSON.parse(input.subarray(0, newline).toString("utf8"));
              const operation = (parsed as Record<string, unknown>)?.operation;
              if (operation === "chat_submit") {
                const skip = chatSubmitSkipReason(parsed as Record<string, unknown>);
                if (skip) {
                  reason = skip;
                } else {
                  const request = parsed as {
                    text: string;
                    idempotency_key: string;
                    attachments?: Array<{ path: string; kind: string; name?: string }>;
                  };
                  const existing = chatTurns.get(request.idempotency_key);
                  if (existing) {
                    outcome = { status: "accepted", reason: "duplicate", turn_id: existing };
                  } else {
                    try {
                      pi.sendUserMessage(buildChatContent(request));
                    } catch {
                      reason = "send-failed";
                    }
                    if (!reason) {
                      const turnId = globalThis.crypto?.randomUUID?.() ??
                        `turn-${Date.now()}-${Math.random().toString(16).slice(2)}`;
                      if (chatTurns.size >= CHAT_TURNS_MAX) {
                        const oldest = chatTurns.keys().next().value;
                        if (oldest !== undefined) chatTurns.delete(oldest);
                      }
                      chatTurns.set(request.idempotency_key, turnId);
                      outcome = { status: "accepted", reason: "", turn_id: turnId };
                    }
                  }
                }
              } else if (operation === "abort") {
                const skip = abortSkipReason(parsed as Record<string, unknown>);
                if (skip) {
                  reason = skip;
                } else {
                  try {
                    currentContext?.abort();
                    outcome = { status: "accepted", reason: "" };
                  } catch {
                    reason = "abort-failed";
                  }
                }
              } else if (operation === "shutdown") {
                const skip = shutdownSkipReason(parsed as Record<string, unknown>);
                if (skip) {
                  reason = skip;
                } else {
                  // Abort-then-shutdown: an in-flight turn is cancelled, then
                  // native teardown runs. No restart follows.
                  try {
                    if (desiredState().state === "working") currentContext?.abort();
                  } catch {
                    // Abort best-effort; teardown proceeds regardless.
                  }
                  acceptedClients.add(client);
                  outcome = { status: "accepted", reason: "shutdown-requested" };
                  try {
                    currentContext?.shutdown();
                  } catch {
                    reason = "shutdown-failed";
                    acceptedClients.delete(client);
                    outcome = undefined;
                  }
                }
              } else {
                reason = restartSkipReason(parsed);
              }
            } catch {
              reason = "invalid-request";
            }
          }
          replied = true;
          if (reason) {
            client.end(`${JSON.stringify({ status: "skipped", reason })}\n`);
            return;
          }
          if (outcome) {
            client.end(`${JSON.stringify(outcome)}\n`);
            return;
          }
          restartAccepted = true;
          acceptedClients.add(client);
          const ctx = currentContext;
          client.end(`${JSON.stringify({ status: "accepted", reason: "shutdown-requested" })}\n`);
          // No await between admission and the public settled-boundary shutdown.
          ctx.shutdown();
        });
      });
      controlServer.on("error", () => {
        removeOwnedSocket();
      });
      controlServer.unref();
      controlServer.listen(endpoint, () => {
        try {
          const stat = fs.lstatSync(endpoint);
          if (!stat.isSocket() || stat.uid !== uid) {
            controlServer?.close();
            return;
          }
          ownedSocket = { path: endpoint, dev: stat.dev, ino: stat.ino };
          fs.chmodSync(endpoint, 0o600);
          if (controlClosed) {
            controlServer?.close();
            removeOwnedSocket();
          }
        } catch {
          controlServer?.close();
          removeOwnedSocket();
        }
      });
    } catch {
      closeControl();
    }
  }

  function clearTimer(timer: ReturnType<typeof setTimeout> | undefined) {
    if (timer) {
      clearTimeout(timer);
    }
  }

  function clearPendingTimers() {
    clearTimer(idleTimer);
    clearTimer(retryTimer);
    idleTimer = undefined;
    retryTimer = undefined;
  }

  function clearFailureState() {
    retryHoldActive = false;
    failureBlocked = false;
    failureMessage = undefined;
  }

  function desiredState() {
    if (explicitBlockedCount > 0) {
      return { state: "blocked" as const, message: explicitBlockedMessage };
    }
    if (waitingCount > 0) {
      return { state: "waiting" as const, message: waitingMessage };
    }
    if (failureBlocked) {
      return { state: "blocked" as const, message: failureMessage };
    }
    if (backgroundAwait) {
      return { state: "waiting" as const, message: "awaiting background work" };
    }
    if (agentActive || retryHoldActive) {
      return { state: "working" as const, message: undefined };
    }
    return { state: "idle" as const, message: undefined };
  }

  function publishState(force = false) {
    const next = desiredState();
    if (!force && next.state === lastState && next.message === lastMessage) {
      return;
    }
    lastState = next.state;
    lastMessage = next.message;
    queueState(next.state, next.message);
  }

  function scheduleIdle() {
    clearPendingTimers();
    clearFailureState();
    idleTimer = setTimeout(() => {
      idleTimer = undefined;
      publishState();
    }, idleDebounceMs);
    idleTimer.unref?.();
  }

  function holdForRetry(message: string) {
    clearPendingTimers();
    retryHoldActive = true;
    failureBlocked = false;
    failureMessage = message;
    publishState();

    retryTimer = setTimeout(() => {
      retryTimer = undefined;
      retryHoldActive = false;
      failureBlocked = true;
      publishState();
    }, retryGraceMs);
    retryTimer.unref?.();
  }

  function activateRootSession(ctx: any, sessionStartSource = "startup"): boolean {
    if (ctx?.hasUI !== true) {
      return false;
    }
    rootSession = true;
    captureContext(ctx);
    startControl();
    void reportSession(sessionStartSource);
    return true;
  }

  function resetSessionState() {
    clearPendingTimers();
    clearFailureState();
    agentActive = false;
    explicitBlockedCount = 0;
    explicitBlockedMessage = undefined;
    waitingCount = 0;
    waitingMessage = undefined;
  }

  function activateWaiting(message: string | undefined) {
    clearPendingTimers();
    waitingCount += 1;
    waitingMessage = message;
    publishState();
  }

  function deactivateWaiting() {
    waitingCount = Math.max(0, waitingCount - 1);
    if (waitingCount === 0) {
      waitingMessage = undefined;
    }
    publishState();
  }

  pi.events.on("herdr:blocked", (data) => {
    if (!rootSession) {
      return;
    }
    if (!data?.active) {
      explicitBlockedCount = Math.max(0, explicitBlockedCount - 1);
      if (explicitBlockedCount === 0) explicitBlockedMessage = undefined;
      publishState();
      return;
    }

    clearPendingTimers();
    explicitBlockedCount += 1;
    explicitBlockedMessage = data.label;
    publishState();
  });

  pi.on("session_start", (_event, ctx) => {
    if (!activateRootSession(ctx)) {
      return;
    }
    // A reload can replace this extension mid-run without emitting another agent_start.
    agentActive = ctx?.isIdle?.() === false;
    publishState(true);
  });

  pi.on("session_switch", (event, ctx) => {
    if (!activateRootSession(ctx, event?.reason || "resume")) {
      return;
    }
    resetSessionState();
    publishState(true);
  });

  pi.on("agent_start", (_event, ctx) => {
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    captureContext(ctx);
    void reportSession();
    clearPendingTimers();
    clearFailureState();
    backgroundAwait = false;
    agentActive = true;
    publishState();
  });

  pi.on("tool_approval_requested", (event, ctx) => {
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    captureContext(ctx);
    const label = event?.reason || `${event?.toolName || "Tool"} approval`;
    activateWaiting(label);
  });

  pi.on("tool_approval_resolved", (_event, ctx) => {
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    captureContext(ctx);
    deactivateWaiting();
  });

  pi.on("tool_execution_start", (event, ctx) => {
    if (event?.toolName !== "ask") {
      return;
    }
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    captureContext(ctx);
    activateWaiting(askWaitingMessage(event.args));
  });

  pi.on("tool_execution_end", (event, ctx) => {
    if (event?.toolName !== "ask") {
      return;
    }
    if (!rootSession && !activateRootSession(ctx)) {
      return;
    }
    captureContext(ctx);
    deactivateWaiting();
  });

  pi.on("agent_end", (event, ctx) => {
    if (!rootSession) {
      return;
    }
    captureContext(ctx);
    if (!agentActive) {
      // OMP can emit duplicate/late end events while auto-retry is already
      // holding the pane in Working. Do not let an unqualified duplicate end
      // cancel the retry hold and publish a false Idle.
      if (event?.willContinue !== true && backgroundAwait) {
        // The awaited background work settled with its own final end.
        backgroundAwait = false;
        scheduleIdle();
      }
      return;
    }
    if (event?.willContinue === true) {
      if (event?.awaitingAsyncWork === true) {
        // The visible reply ended; OMP stays busy only awaiting background
        // results. Settle the foreground turn and surface the background
        // wait as a distinct waiting state instead of yellow Working.
        agentActive = false;
        backgroundAwait = true;
        clearPendingTimers();
        publishState(true);
        return;
      }
      // A continuation is already scheduled, so this end is not a settle.
      // Older builds omit the field and fall through as before.
      return;
    }

    agentActive = false;
    backgroundAwait = false;

    const retryableMessage = retryableErrorMessage(event);
    if (retryableMessage) {
      holdForRetry(retryableMessage);
      return;
    }

    scheduleIdle();
  });

  pi.on("session_shutdown", () => {
    closeControl();
    if (rootSession) {
      clearPendingTimers();
    }
  });
}
