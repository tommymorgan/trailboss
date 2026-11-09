// installed by trailboss
// managed by trailboss; reinstalling or updating the integration overwrites this file.
// add custom hooks/plugins beside this file instead of editing it.
// TRAILBOSS_INTEGRATION_ID=pi
// TRAILBOSS_INTEGRATION_VERSION=11
// @ts-nocheck

import { randomUUID } from "node:crypto";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";

const HERDR_ENV = process.env.HERDR_ENV;
const socketPath = process.env.HERDR_SOCKET_PATH;
const socketEndpoint =
  process.platform === "win32" && socketPath ? `\\\\.\\pipe\\${socketPath}` : socketPath;
const paneId = process.env.HERDR_PANE_ID;
const source = "herdr:pi";

function enabled() {
  return HERDR_ENV === "1" && !!socketPath && !!paneId;
}

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

async function sendRequest(request: unknown): Promise<void> {
  if (await sendRequestAttempt(request, 500)) {
    return;
  }
  await sendRequestAttempt(request, 1500);
}

type AgentState = "working" | "waiting" | "blocked" | "idle";

type QueuedState = {
  state: AgentState;
  message?: string;
  seq: number;
};

let reportSeq = Date.now() * 1000;
let currentAgentSessionId: string | undefined;
let currentAgentSessionPath: string | undefined;

function nextReportSeq(): number {
  reportSeq += 1;
  return reportSeq;
}

function updateSessionRef(ctx: any): void {
  try {
    const file = ctx?.sessionManager?.getSessionFile?.();
    currentAgentSessionPath =
      typeof file === "string" &&
      (path.posix.isAbsolute(file) || path.win32.isAbsolute(file))
        ? file
        : undefined;
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

function currentSessionRef(): Record<string, unknown> | undefined {
  if (currentAgentSessionPath) {
    return { agent_session_path: currentAgentSessionPath };
  }
  if (currentAgentSessionId) {
    return { agent_session_id: currentAgentSessionId };
  }
  return undefined;
}

function reportSession(sessionStartSource?: string): Promise<void> {
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
      agent: "pi",
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
      agent: "pi",
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

export default function (pi) {
  if (!enabled()) {
    return;
  }

  let agentActive = false;
  let blockedCount = 0;
  let blockedMessage: string | undefined;
  let busyCount = 0;
  let lastState: AgentState | undefined;
  let lastMessage: string | undefined;
  let rootSession = false;
  let currentContext: any;
  let controlServer: net.Server | undefined;
  let ownedSocket: { path: string; dev: number; ino: number } | undefined;
  let restartAccepted = false;
  let controlClosed = false;
  const controlClients = new Set<net.Socket>();
  const acceptedClients = new Set<net.Socket>();
  const CONTROL_FRAME_LIMIT = 8192;
  // Well under Core's 3s control step, which also covers the final recheck.
  const SUBAGENT_STATUS_DEADLINE_MS = 1000;

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

  // Identity admission shared by launch-info and restart-if-idle.
  function baseSkipReason(request: unknown, operation: string): string | undefined {
    if (!request || typeof request !== "object" || Array.isArray(request)) return "invalid-request";
    if (request.operation !== operation || !Number.isSafeInteger(request.pid) ||
        typeof request.pane_id !== "string" || typeof request.session_path !== "string" ||
        !path.posix.isAbsolute(request.session_path)) return "invalid-request";
    if (request.pid !== process.pid) return "pid-mismatch";
    if (request.pane_id !== paneId) return "pane-mismatch";
    if (controlClosed || !rootSession || !currentContext) return "interactive-context-unavailable";
    // Re-read the live manager: cached report identity is not admission authority.
    updateSessionRef(currentContext);
    if (!currentAgentSessionPath || request.session_path !== currentAgentSessionPath) {
      return "session-mismatch";
    }
    return undefined;
  }

  function restartSkipReason(request: unknown): string | undefined {
    const base = baseSkipReason(request, "restart-if-idle");
    if (base) return base;
    if (restartAccepted) return "restart-already-accepted";
    if (desiredState().state !== "idle") return "reporter-not-idle";
    const ctx = currentContext;
    if (typeof ctx.isIdle !== "function" || typeof ctx.hasPendingMessages !== "function" ||
        typeof ctx.ui?.getEditorText !== "function" || typeof ctx.shutdown !== "function" ||
        typeof pi.getAllTools !== "function") {
      return "runtime-capability-unavailable";
    }
    try {
      if (ctx.isIdle() !== true) return "runtime-not-idle";
      if (ctx.hasPendingMessages() !== false) return "pending-messages";
      if (ctx.ui.getEditorText() !== "") return "editor-not-empty";
    } catch {
      return "runtime-state-unavailable";
    }
    return undefined;
  }

  // pi-subagents async runs are detached from the foreground job, so only its
  // in-process status RPC can tell whether a restart would stop them.
  function subagentToolLoaded(): boolean {
    return pi.getAllTools().some((tool) => tool?.name === "subagent");
  }

  function backgroundRunsSkipReason(): Promise<string | undefined> {
    return new Promise((resolve) => {
      let settled = false;
      let unsubscribe: (() => void) | undefined;
      const finish = (reason: string | undefined) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        try {
          unsubscribe?.();
        } catch {
          // The reply listener is single-use either way.
        }
        resolve(reason);
      };
      const timer = setTimeout(() => finish("async-state-unavailable"), SUBAGENT_STATUS_DEADLINE_MS);
      timer.unref?.();
      try {
        const requestId = randomUUID();
        unsubscribe = pi.events.on(`subagents:rpc:v1:reply:${requestId}`, (reply) => {
          const totalActive = reply?.success === true ? reply.data?.fleet?.totalActive : undefined;
          if (!Number.isSafeInteger(totalActive) || totalActive < 0) {
            finish("async-state-unavailable");
          } else {
            finish(totalActive > 0 ? "async-jobs-running" : undefined);
          }
        });
        pi.events.emit("subagents:rpc:v1:request", {
          version: 1,
          requestId,
          method: "status",
          params: {},
        });
      } catch {
        finish("async-state-unavailable");
      }
    });
  }

  function handleControlRequest(request: unknown): Record<string, unknown> | "restart" {
    if (request?.operation === "launch-info") {
      const reason = baseSkipReason(request, "launch-info");
      if (reason) return { status: "skipped", reason };
      return {
        status: "accepted",
        reason: "",
        result: { argv: [...process.argv], exec_path: process.execPath },
      };
    }
    const reason = restartSkipReason(request);
    return reason ? { status: "skipped", reason } : "restart";
  }

  function startControl() {
    if (controlServer || controlClosed || process.platform !== "linux" || !process.geteuid) return;
    const uid = process.geteuid();
    const directory = `/tmp/trailboss-pi-control-${uid}`;
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
        const deadline = setTimeout(() => client.destroy(), 2000);
        deadline.unref();
        client.on("error", () => client.destroy());
        client.on("close", () => {
          clearTimeout(deadline);
          controlClients.delete(client);
          acceptedClients.delete(client);
        });
        let input = Buffer.alloc(0);
        let replied = false;
        const reply = (body: Record<string, unknown>) => {
          if (!client.destroyed) client.end(`${JSON.stringify(body)}\n`);
        };
        client.on("data", (chunk) => {
          if (replied) return;
          if (input.length + chunk.length > CONTROL_FRAME_LIMIT) {
            replied = true;
            reply({ status: "skipped", reason: "request-too-large" });
            return;
          }
          input = Buffer.concat([input, chunk]);
          const newline = input.indexOf(10);
          if (newline < 0) return;
          replied = true;
          if (newline !== input.length - 1) {
            reply({ status: "skipped", reason: "invalid-frame" });
            return;
          }
          let request: unknown;
          try {
            request = JSON.parse(input.subarray(0, newline).toString("utf8"));
          } catch {
            reply({ status: "skipped", reason: "invalid-request" });
            return;
          }
          const outcome = handleControlRequest(request);
          if (outcome !== "restart") {
            reply(outcome);
            return;
          }
          const acceptRestart = () => {
            // Every synchronous gate runs again here, with no await before shutdown.
            const reason = restartSkipReason(request);
            if (reason) {
              reply({ status: "skipped", reason });
              return;
            }
            restartAccepted = true;
            acceptedClients.add(client);
            const ctx = currentContext;
            reply({ status: "accepted", reason: "shutdown-requested" });
            ctx.shutdown();
          };
          let backgroundGate: boolean;
          try {
            backgroundGate = subagentToolLoaded();
          } catch {
            reply({ status: "skipped", reason: "runtime-state-unavailable" });
            return;
          }
          if (!backgroundGate) {
            acceptRestart();
            return;
          }
          void backgroundRunsSkipReason().then((reason) => {
            if (reason) reply({ status: "skipped", reason });
            else acceptRestart();
          });
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

  function desiredState() {
    if (blockedCount > 0) {
      return { state: "blocked" as const, message: blockedMessage };
    }
    if (agentActive) {
      return { state: "working" as const, message: undefined };
    }
    if (busyCount > 0) {
      return { state: "waiting" as const, message: "awaiting background work" };
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

  pi.events.on("herdr:blocked", (data) => {
    if (!rootSession) {
      return;
    }
    if (!data?.active) {
      blockedCount = Math.max(0, blockedCount - 1);
      if (blockedCount === 0) {
        blockedMessage = undefined;
      }
      publishState();
      return;
    }

    blockedCount += 1;
    blockedMessage = data.label;
    publishState();
  });

  let busyPublishPending = false;

  // pi-subagents raises herdr:busy while async runs remain after the parent turn.
  // A label change clears and re-raises in one tick; publishing once per microtask
  // keeps that swap from reporting a transient idle. After a reload, pi-subagents
  // can restore busy before this session_start runs; count it now, report it there.
  pi.events.on("herdr:busy", (data) => {
    busyCount = data?.active ? busyCount + 1 : Math.max(0, busyCount - 1);
    if (!rootSession || busyPublishPending) {
      return;
    }
    busyPublishPending = true;
    queueMicrotask(() => {
      busyPublishPending = false;
      publishState();
    });
  });

  pi.on("session_start", async (event, ctx) => {
    // TUI only: RPC/JSON/print modes are headless (no PTY trailboss can display),
    // and RPC still reports hasUI=true, so mode is the reliable gate.
    if (ctx?.mode !== "tui") {
      return;
    }
    rootSession = true;
    currentContext = ctx;
    updateSessionRef(ctx);
    startControl();
    await reportSession(event?.reason);
    // A reload can replace this extension mid-run without emitting another agent_start.
    agentActive = ctx?.isIdle?.() === false;
    publishState(true);
  });

  pi.on("agent_start", (_event, ctx) => {
    if (!rootSession) {
      return;
    }
    currentContext = ctx;
    updateSessionRef(ctx);
    void reportSession();
    agentActive = true;
    publishState();
  });

  pi.on("agent_settled", (_event, ctx) => {
    if (!rootSession || ctx?.isIdle?.() !== true) {
      return;
    }

    currentContext = ctx;
    agentActive = false;
    publishState();
  });

  pi.on("session_shutdown", () => {
    closeControl();
  });
}
