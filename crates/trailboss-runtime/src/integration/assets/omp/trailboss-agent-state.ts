// installed by trailboss
// managed by trailboss; reinstalling or updating the integration overwrites this file.
// add custom hooks/plugins beside this file instead of editing it.
// TRAILBOSS_INTEGRATION_ID=omp
// TRAILBOSS_INTEGRATION_VERSION=12
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
// OMP marks every shell it spawns with OMPCODE=1. A nested `omp` launched from
// a parent session's shell inherits it, so that process is not the pane's root
// agent and must not report its short-lived session over the parent's.
const nestedOmpSession = process.env.OMPCODE === "1";

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
          const tooLarge = input.length + chunk.length > 8192;
          if (!tooLarge) input = Buffer.concat([input, chunk]);
          const newline = input.indexOf(10);
          let reason: string | undefined;
          if (tooLarge) reason = "request-too-large";
          else if (newline < 0) return;
          else if (newline !== input.length - 1) reason = "invalid-frame";
          else {
            try {
              reason = restartSkipReason(JSON.parse(input.subarray(0, newline).toString("utf8")));
            } catch {
              reason = "invalid-request";
            }
          }
          replied = true;
          if (reason) {
            client.end(`${JSON.stringify({ status: "skipped", reason })}\n`);
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
      return;
    }
    if (event?.willContinue === true) {
      // A continuation is already scheduled, so this end is not a settle.
      // Older builds omit the field and fall through as before.
      return;
    }

    agentActive = false;

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
