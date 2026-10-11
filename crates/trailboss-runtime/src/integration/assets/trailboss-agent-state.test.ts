import { afterEach, expect, test } from "bun:test";
import { rm } from "node:fs/promises";
import { existsSync, lstatSync, symlinkSync, unlinkSync } from "node:fs";
import net, { createServer, type Server } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

const originalPlatform = process.platform;
const originalArgv = process.argv;
const originalCreateConnection = net.createConnection;
const originalEnvironment = {
  HERDR_ENV: process.env.HERDR_ENV,
  TRAILBOSS_OMP_IDLE_DEBOUNCE_MS: process.env.TRAILBOSS_OMP_IDLE_DEBOUNCE_MS,
  TRAILBOSS_OMP_RETRY_GRACE_MS: process.env.TRAILBOSS_OMP_RETRY_GRACE_MS,
  HERDR_PANE_ID: process.env.HERDR_PANE_ID,
  HERDR_SOCKET_PATH: process.env.HERDR_SOCKET_PATH,
  OMPCODE: process.env.OMPCODE,
};

let server: Server | undefined;
let socketPath: string | undefined;
let importCounter = 0;
const extensionHarnesses: Array<Map<string, Handler>> = [];

afterEach(async () => {
  for (const handlers of extensionHarnesses.splice(0)) {
    await handlers.get("session_shutdown")?.({}, undefined);
  }
  await new Promise<void>((resolve, reject) => {
    if (!server) {
      resolve();
      return;
    }
    server.close((error) => (error ? reject(error) : resolve()));
  });
  server = undefined;

  if (socketPath) {
    await rm(socketPath, { force: true });
    socketPath = undefined;
  }

  Object.defineProperty(process, "platform", { value: originalPlatform });
  net.createConnection = originalCreateConnection;
  process.argv = originalArgv;
  for (const [name, value] of Object.entries(originalEnvironment)) {
    if (value === undefined) {
      delete process.env[name];
    } else {
      process.env[name] = value;
    }
  }
});

const integrations = [
  { name: "Pi", modulePath: "./pi/trailboss-agent-state.ts" },
  { name: "Oh My Pi", modulePath: "./omp/trailboss-agent-state.ts" },
] as const;

const socketPlugins = [
  {
    name: "OpenCode",
    modulePath: "./opencode/trailboss-agent-state.js",
    sessionID: "opencode-session",
  },
  { name: "Kilo", modulePath: "./kilo/trailboss-agent-state.js", sessionID: "kilo-session" },
] as const;

function importFresh(modulePath: string) {
  importCounter += 1;
  return import(`${modulePath}?test=${importCounter}`);
}

type Handler = (event: unknown, context: unknown) => unknown;

function createExtensionHarness() {
  const handlers = new Map<string, Handler>();
  const eventHandlers = new Map<string, Handler>();
  extensionHarnesses.push(handlers);
  return {
    handlers,
    eventHandlers,
    pi: {
      on(event: string, handler: Handler) {
        handlers.set(event, handler);
      },
      events: {
        on(event: string, handler: Handler) {
          eventHandlers.set(event, handler);
          return () => {};
        },
      },
    },
  };
}

function configureIntegrationEnvironment(recordingSocketPath: string) {
  // Tests may run inside an OMP shell; nested-session cases opt in explicitly.
  delete process.env.OMPCODE;
  process.env.HERDR_ENV = "1";
  process.env.HERDR_SOCKET_PATH = recordingSocketPath;
  process.env.HERDR_PANE_ID = "test:p1";
}

function captureConnectionEndpoint() {
  let connectedEndpoint: unknown;
  net.createConnection = ((...args: unknown[]) => {
    connectedEndpoint = args[0];
    return Reflect.apply(originalCreateConnection, net, args);
  }) as typeof net.createConnection;
  return () => connectedEndpoint;
}

async function startRecordingServer(name: string): Promise<unknown[]> {
  const recordingSocketPath = join(tmpdir(), `trailboss-${name}-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });

  const requests: unknown[] = [];
  const recordingServer = createServer((socket) => {
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline === -1) {
        return;
      }
      requests.push(JSON.parse(input.slice(0, newline)));
      socket.end("{}\n");
    });
  });
  server = recordingServer;
  await new Promise<void>((resolve, reject) => {
    recordingServer.once("error", reject);
    recordingServer.listen(originalPlatform === "win32" ? `\\\\.\\pipe\\${recordingSocketPath}` : recordingSocketPath, resolve);
  });
  configureIntegrationEnvironment(recordingSocketPath);
  return requests;
}

for (const socketPlugin of socketPlugins) {
  test(`${socketPlugin.name} maps the Windows socket marker path to a named pipe endpoint`, async () => {
    const markerPath = `trailboss-${socketPlugin.name.toLowerCase()}-${process.pid}.sock`;
    configureIntegrationEnvironment(markerPath);
    Object.defineProperty(process, "platform", { value: "win32" });
    const connectedEndpoint = captureConnectionEndpoint();

    process.argv = ["bun", "/$bunfs/root/src/index.js", "run"];
    const { TrailbossAgentStatePlugin } = await importFresh(socketPlugin.modulePath);
    const plugin = await TrailbossAgentStatePlugin();
    await plugin.event({
      event: {
        type: "session.updated",
        properties: { sessionID: socketPlugin.sessionID },
      },
    });

    expect(connectedEndpoint()).toBe(`\\\\.\\pipe\\${markerPath}`);
  });
}

test("OpenCode stays disabled without the Trailboss socket environment", async () => {
  process.env.HERDR_ENV = "1";
  process.env.HERDR_PANE_ID = "test:p1";
  delete process.env.HERDR_SOCKET_PATH;

  const { TrailbossAgentStatePlugin } = await importFresh("./opencode/trailboss-agent-state.js");

  expect(await TrailbossAgentStatePlugin()).toEqual({});
});

for (const integration of integrations) {
  test(`${integration.name} maps the Windows socket marker path to a named pipe endpoint`, async () => {
    const markerPath = `trailboss-${integration.name.toLowerCase().replaceAll(" ", "-")}-${process.pid}.sock`;
    configureIntegrationEnvironment(markerPath);
    Object.defineProperty(process, "platform", { value: "win32" });
    const connectedEndpoint = captureConnectionEndpoint();
    const { handlers, pi } = createExtensionHarness();

    const { default: install } = await importFresh(integration.modulePath);
    install(pi);
    await handlers.get("session_start")?.(
      { reason: "startup" },
      {
        hasUI: true,
        mode: "tui",
        isIdle: () => true,
        sessionManager: {
          getSessionFile: () => undefined,
          getSessionId: () => "test-session",
        },
      },
    );

    expect(connectedEndpoint()).toBe(`\\\\.\\pipe\\${markerPath}`);
  });

  test(`${integration.name} reload preserves working state when the agent is active`, async () => {
    const requests = await startRecordingServer(
      integration.name.toLowerCase().replaceAll(" ", "-"),
    );
    const { handlers, pi } = createExtensionHarness();

    const { default: install } = await importFresh(integration.modulePath);
    install(pi);

    const sessionStart = handlers.get("session_start");
    expect(sessionStart).toBeDefined();
    await sessionStart?.(
      { reason: "reload" },
      {
        hasUI: true,
        mode: "tui",
        isIdle: () => false,
        sessionManager: {
          getSessionFile: () => undefined,
          getSessionId: () => undefined,
        },
      },
    );

    const reportedState = () => {
      for (const request of requests) {
        if (!isRecord(request) || request.method !== "pane.report_agent") {
          continue;
        }
        const params = request.params;
        if (isRecord(params) && typeof params.state === "string") {
          return params.state;
        }
      }
      return undefined;
    };

    const deadline = Date.now() + 1_000;
    while (Date.now() < deadline && reportedState() === undefined) {
      await Bun.sleep(5);
    }

    expect(reportedState()).toBe("working");
  });
}

test("OMP ignores nested sessions launched inside another OMP shell", async () => {
  const requests = await startRecordingServer("omp-nested");
  process.env.OMPCODE = "1";
  const { handlers, pi } = createExtensionHarness();

  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(pi);

  // OMP sets OMPCODE on every shell it spawns. A nested `omp` inherits it and
  // must not claim the pane's session for its short-lived conversation.
  expect(handlers.size).toBe(0);
  await handlers.get("session_start")?.(
    { reason: "startup" },
    {
      hasUI: true,
      isIdle: () => true,
      sessionManager: {
        getSessionFile: () => "/tmp/omp-nested.jsonl",
        getSessionId: () => "omp-nested",
      },
    },
  );
  await Bun.sleep(25);

  expect(requests).toEqual([]);
});

test("OMP accepts POSIX and Windows session paths", async () => {
  const { isAbsoluteSessionPath } = await importFresh("./omp/trailboss-agent-state.ts");

  expect(isAbsoluteSessionPath("/tmp/omp-session.jsonl")).toBe(true);
  expect(isAbsoluteSessionPath("C:\\Users\\User\\.omp\\agent\\sessions\\omp-session.jsonl")).toBe(
    true,
  );
  expect(isAbsoluteSessionPath("C:/Users/User/.omp/agent/sessions/omp-session.jsonl")).toBe(true);
  expect(isAbsoluteSessionPath("relative/omp-session.jsonl")).toBe(false);
});

test("Pi reports a Windows session path", async () => {
  const requests = await startRecordingServer("pi-windows-session-path");
  const { handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./pi/trailboss-agent-state.ts");
  install(pi);

  const sessionPath = "C:\\Users\\User\\.pi\\agent\\sessions\\pi-session.jsonl";
  await handlers.get("session_start")?.(
    { reason: "startup" },
    {
      ...piContext(() => true),
      sessionManager: {
        getSessionFile: () => sessionPath,
        getSessionId: () => "pi-session",
      },
    },
  );
  await waitFor(() => requests.length === 2);

  expect(requests.map(requestSessionPath)).toEqual([sessionPath, sessionPath]);
});

test("Pi reports idle only after the agent settles", async () => {
  const requests = await startRecordingServer("pi-settled");
  const { handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./pi/trailboss-agent-state.ts");
  install(pi);

  expect(completionHandlers(handlers)).toEqual(["agent_settled"]);
  let idle = true;
  const context = piContext(() => idle);
  await handlers.get("session_start")?.({ reason: "startup" }, context);
  await waitFor(() => requestStates(requests).length === 1);

  idle = false;
  handlers.get("agent_start")?.({}, context);
  await waitFor(() => requestStates(requests).length === 2);
  expect(requestStates(requests)).toEqual(["idle", "working"]);
  expect(handlers.has("agent_end")).toBe(false);

  const requestCountBeforeStaleSettlement = requests.length;
  handlers.get("agent_settled")?.({}, context);
  await Bun.sleep(25);
  expect(requests).toHaveLength(requestCountBeforeStaleSettlement);
  expect(requestStates(requests)).toEqual(["idle", "working"]);

  idle = true;
  handlers.get("agent_settled")?.({}, context);
  await waitFor(() => requestStates(requests).length === 3);
  expect(requestStates(requests)).toEqual(["idle", "working", "idle"]);
});

test("Pi ignores RPC sessions even when UI APIs are available", async () => {
  const requests = await startRecordingServer("pi-rpc");
  const { handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./pi/trailboss-agent-state.ts");
  install(pi);

  const context = {
    ...piContext(() => true),
    hasUI: true,
    mode: "rpc",
  };
  await handlers.get("session_start")?.({ reason: "startup" }, context);
  handlers.get("agent_start")?.({}, context);
  handlers.get("agent_settled")?.({}, context);
  await Bun.sleep(25);

  expect(requests).toEqual([]);
});

test("Pi settlement preserves explicit blocked-state precedence", async () => {
  const requests = await startRecordingServer("pi-settled-blocked");
  const { eventHandlers, handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./pi/trailboss-agent-state.ts");
  install(pi);

  let idle = true;
  const context = piContext(() => idle);
  await handlers.get("session_start")?.({ reason: "startup" }, context);
  await waitFor(() => requestStates(requests).length === 1);
  idle = false;
  handlers.get("agent_start")?.({}, context);
  await waitFor(() => requestStates(requests).length === 2);
  eventHandlers.get("herdr:blocked")?.({ active: true, label: "approval" }, context);
  await waitFor(() => requestStates(requests).length === 3);

  idle = true;
  handlers.get("agent_settled")?.({}, context);
  await Bun.sleep(25);
  expect(requestStates(requests)).toEqual(["idle", "working", "blocked"]);

  eventHandlers.get("herdr:blocked")?.({ active: false }, context);
  await waitFor(() => requestStates(requests).length === 4);
  expect(requestStates(requests)).toEqual(["idle", "working", "blocked", "idle"]);
});

test("Pi reports the session replacement source", async () => {
  const requests = await startRecordingServer("pi-session-source");
  const { handlers, pi } = createExtensionHarness();

  const { default: install } = await importFresh("./pi/trailboss-agent-state.ts");
  install(pi);

  const sessionStart = handlers.get("session_start");
  expect(sessionStart).toBeDefined();
  await sessionStart?.(
    { reason: "new" },
    {
      hasUI: true,
      mode: "tui",
      isIdle: () => true,
      sessionManager: {
        getSessionFile: () => "/tmp/pi-new.jsonl",
        getSessionId: () => "pi-new",
      },
    },
  );

  const reportedSession = () =>
    requests.find((request) => isRecord(request) && request.method === "pane.report_agent_session");
  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline && reportedSession() === undefined) {
    await Bun.sleep(5);
  }

  const request = reportedSession();
  expect(request).toBeDefined();
  expect(isRecord(request) && isRecord(request.params) ? request.params.session_start_source : null)
    .toBe("new");
});

test("Pi waits for a replacement session report before publishing state", async () => {
  const recordingSocketPath = join(tmpdir(), `trailboss-pi-session-order-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });

  const requests: unknown[] = [];
  let acknowledgeSessionReport: (() => void) | undefined;
  const recordingServer = createServer((socket) => {
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline === -1) {
        return;
      }
      const request = JSON.parse(input.slice(0, newline));
      requests.push(request);
      if (isRecord(request) && request.method === "pane.report_agent_session") {
        acknowledgeSessionReport = () => socket.end("{}\n");
        return;
      }
      socket.end("{}\n");
    });
  });
  server = recordingServer;
  await new Promise<void>((resolve, reject) => {
    recordingServer.once("error", reject);
    recordingServer.listen(originalPlatform === "win32" ? `\\\\.\\pipe\\${recordingSocketPath}` : recordingSocketPath, resolve);
  });

  configureIntegrationEnvironment(recordingSocketPath);
  const { handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./pi/trailboss-agent-state.ts");
  install(pi);

  const sessionStart = handlers.get("session_start");
  expect(sessionStart).toBeDefined();
  const sessionStartResult = sessionStart?.(
    { reason: "new" },
    {
      hasUI: true,
      mode: "tui",
      isIdle: () => false,
      sessionManager: {
        getSessionFile: () => "/tmp/pi-new.jsonl",
        getSessionId: () => "pi-new",
      },
    },
  );

  const deadline = Date.now() + 1_000;
  while (Date.now() < deadline && acknowledgeSessionReport === undefined) {
    await Bun.sleep(5);
  }
  expect(acknowledgeSessionReport).toBeDefined();
  expect(
    requests.some((request) => isRecord(request) && request.method === "pane.report_agent"),
  ).toBe(false);

  acknowledgeSessionReport?.();
  await sessionStartResult;

  const stateDeadline = Date.now() + 1_000;
  while (
    Date.now() < stateDeadline &&
    !requests.some((request) => isRecord(request) && request.method === "pane.report_agent")
  ) {
    await Bun.sleep(5);
  }
  expect(requests.map((request) => (isRecord(request) ? request.method : undefined))).toEqual([
    "pane.report_agent_session",
    "pane.report_agent",
  ]);
});

async function startDroppedFirstResponseServer(name: string) {
  const recordingSocketPath = join(tmpdir(), `trailboss-${name}-${process.pid}.sock`);
  socketPath = recordingSocketPath;
  await rm(recordingSocketPath, { force: true });

  let connectionCount = 0;
  const attemptedRequests: unknown[] = [];
  const deliveredRequests: unknown[] = [];
  const recordingServer = createServer((socket) => {
    connectionCount += 1;
    const connectionNumber = connectionCount;
    let input = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline === -1) {
        return;
      }
      const request = JSON.parse(input.slice(0, newline));
      attemptedRequests.push(request);
      if (connectionNumber === 1) {
        return;
      }
      deliveredRequests.push(request);
      socket.end("{}\n");
    });
  });
  server = recordingServer;
  await new Promise<void>((resolve, reject) => {
    recordingServer.once("error", reject);
    recordingServer.listen(originalPlatform === "win32" ? `\\\\.\\pipe\\${recordingSocketPath}` : recordingSocketPath, resolve);
  });

  configureIntegrationEnvironment(recordingSocketPath);
  return {
    attemptedRequests,
    deliveredRequests,
    connectionCount: () => connectionCount,
  };
}

test("Oh My Pi retries working before a queued idle state", async () => {
  const { attemptedRequests } = await startDroppedFirstResponseServer("omp-retry");
  process.env.TRAILBOSS_OMP_IDLE_DEBOUNCE_MS = "0";
  const { handlers, pi } = createExtensionHarness();

  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(pi);

  const context = {
    hasUI: true,
    isIdle: () => false,
    sessionManager: {
      getSessionFile: () => undefined,
      getSessionId: () => undefined,
    },
  };
  handlers.get("session_start")?.({ reason: "startup" }, context);
  handlers.get("agent_end")?.({ messages: [] }, context);

  const deadline = Date.now() + 2_500;
  while (Date.now() < deadline && attemptedRequests.length < 3) {
    await Bun.sleep(5);
  }

  expect(attemptedRequests).toHaveLength(3);
  expect(attemptedRequests[1]).toEqual(attemptedRequests[0]);
  expect(requestState(attemptedRequests[0])).toBe("working");
  expect(requestState(attemptedRequests[2])).toBe("idle");
});

test("Oh My Pi keeps working when a turn ends with a scheduled continuation", async () => {
  const requests = await startRecordingServer("omp-will-continue");
  process.env.TRAILBOSS_OMP_IDLE_DEBOUNCE_MS = "0";
  const { handlers, pi } = createExtensionHarness();

  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(pi);

  let idle = true;
  const context = {
    hasUI: true,
    isIdle: () => idle,
    sessionManager: {
      getSessionFile: () => undefined,
      getSessionId: () => undefined,
    },
  };

  handlers.get("session_start")?.({ reason: "startup" }, context);
  await waitFor(() => requestStates(requests).length === 1);

  idle = false;
  handlers.get("agent_start")?.({}, context);
  await waitFor(() => requestStates(requests).length === 2);
  expect(requestStates(requests)).toEqual(["idle", "working"]);

  // OMP already scheduled an automatic continuation, so this loop end is not a
  // user-visible settle and must not publish idle. See issue #2851.
  handlers.get("agent_end")?.({ messages: [], willContinue: true }, context);
  await Bun.sleep(50);
  expect(requestStates(requests)).toEqual(["idle", "working"]);

  // The real terminal end still settles the pane.
  idle = true;
  handlers.get("agent_end")?.({ messages: [] }, context);
  await waitFor(() => requestStates(requests).length === 3);
  expect(requestStates(requests)).toEqual(["idle", "working", "idle"]);
});

test("Pi retries working state after an unanswered socket attempt", async () => {
  const { attemptedRequests, deliveredRequests, connectionCount } =
    await startDroppedFirstResponseServer("pi-retry");
  const { handlers, pi } = createExtensionHarness();

  const { default: install } = await importFresh("./pi/trailboss-agent-state.ts");
  install(pi);

  const sessionStart = handlers.get("session_start");
  expect(sessionStart).toBeDefined();
  await sessionStart?.(
    { reason: "startup" },
    {
      hasUI: true,
      mode: "tui",
      isIdle: () => false,
      sessionManager: {
        getSessionFile: () => undefined,
        getSessionId: () => undefined,
      },
    },
  );

  const reportedWorking = () =>
    deliveredRequests.some((request) => {
      if (!isRecord(request) || request.method !== "pane.report_agent") {
        return false;
      }
      const params = request.params;
      return isRecord(params) && params.state === "working";
    });

  const deadline = Date.now() + 2_500;
  while (Date.now() < deadline && !reportedWorking()) {
    await Bun.sleep(5);
  }

  expect(connectionCount()).toBeGreaterThanOrEqual(2);
  expect(attemptedRequests.length).toBeGreaterThanOrEqual(2);
  expect(attemptedRequests[1]).toEqual(attemptedRequests[0]);
  expect(reportedWorking()).toBe(true);
});

test("OMP overlapping ask and approval wait until both resolve before resuming", async () => {
  const requests = await startRecordingServer("omp-overlapping-waits");
  process.env.TRAILBOSS_OMP_IDLE_DEBOUNCE_MS = "0";
  const { handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(pi);
  const context = piContext(() => false);
  handlers.get("session_start")?.({}, context);
  await waitFor(() => requestStates(requests).length === 1);
  handlers.get("tool_approval_requested")?.({ reason: "Approve command" }, context);
  await waitFor(() => requestStates(requests).length === 2);
  handlers.get("tool_execution_start")?.({ toolName: "ask", args: { questions: [{ question: "Choose target" }] } }, context);
  await waitFor(() => requestStates(requests).length === 3);
  handlers.get("tool_approval_resolved")?.({}, context);
  await Bun.sleep(25);
  expect(requestStates(requests)).toEqual(["working", "waiting", "waiting"]);
  handlers.get("tool_execution_end")?.({ toolName: "ask" }, context);
  await waitFor(() => requestStates(requests).length === 4);
  expect(requestStates(requests).at(-1)).toBe("working");
  handlers.get("agent_end")?.({ messages: [] }, context);
  await waitFor(() => requestStates(requests).length === 5);
  expect(requestStates(requests).at(-1)).toBe("idle");
});

test("OMP resolving an idle approval returns to idle without hiding an explicit blocker", async () => {
  const requests = await startRecordingServer("omp-idle-wait");
  const { eventHandlers, handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(pi);
  const context = piContext(() => true);
  handlers.get("session_start")?.({}, context);
  await waitFor(() => requestStates(requests).length === 1);
  handlers.get("tool_approval_requested")?.({ reason: "Approve command" }, context);
  await waitFor(() => requestStates(requests).length === 2);
  eventHandlers.get("herdr:blocked")?.({ active: true, label: "Missing dependency" }, context);
  await waitFor(() => requestStates(requests).length === 3);
  handlers.get("tool_approval_resolved")?.({}, context);
  await Bun.sleep(25);
  expect(requestStates(requests)).toEqual(["idle", "waiting", "blocked"]);
  eventHandlers.get("herdr:blocked")?.({ active: false }, context);
  await waitFor(() => requestStates(requests).length === 4);
  expect(requestStates(requests).at(-1)).toBe("idle");
});

test("OMP retry holds working before publishing failure blocked and recovers on restart", async () => {
  const requests = await startRecordingServer("omp-failure-blocked");
  process.env.TRAILBOSS_OMP_RETRY_GRACE_MS = "50";
  const { handlers, pi } = createExtensionHarness();
  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(pi);
  const context = piContext(() => false);
  handlers.get("session_start")?.({}, context);
  await waitFor(() => requestStates(requests).length === 1);
  handlers.get("agent_end")?.({ messages: [{ role: "assistant", stopReason: "error", errorMessage: "503 service unavailable" }] }, context);
  expect(requestStates(requests)).toEqual(["working"]);
  await waitFor(() => requestStates(requests).length === 2);
  expect(requestStates(requests)).toEqual(["working", "blocked"]);
  handlers.get("agent_start")?.({}, context);
  await waitFor(() => requestStates(requests).length === 3);
  expect(requestStates(requests).at(-1)).toBe("working");
});

for (const socketPlugin of socketPlugins) {
  test(socketPlugin.name + " distinguishes human waiting from failure blockers", async () => {
    const requests = await startRecordingServer(socketPlugin.name.toLowerCase() + "-waiting");
    process.argv = ["bun", "/$bunfs/root/src/index.js", "run"];
    const { TrailbossAgentStatePlugin } = await importFresh(socketPlugin.modulePath);
    const plugin = await TrailbossAgentStatePlugin();
    for (const type of ["permission.asked", "permission.replied", "question.asked", "question.rejected", "session.error", "session.idle"]) {
      await plugin.event({ event: { type, properties: { sessionID: socketPlugin.sessionID } } });
    }
    expect(requestStates(requests)).toEqual(["waiting", "working", "waiting", "working", "blocked", "idle"]);
  });
}

function completionHandlers(handlers: Map<string, Handler>): string[] {
  return ["agent_end", "agent_settled"].filter((event) => handlers.has(event));
}

function piContext(isIdle: () => boolean) {
  return {
    hasUI: true,
    mode: "tui",
    isIdle,
    sessionManager: {
      getSessionFile: () => undefined,
      getSessionId: () => undefined,
    },
  };
}

function requestStates(requests: unknown[]): unknown[] {
  return requests
    .filter((request) => isRecord(request) && request.method === "pane.report_agent")
    .map(requestState);
}

async function waitFor(predicate: () => boolean, timeoutMs = 1_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline && !predicate()) {
    await Bun.sleep(5);
  }
  expect(predicate()).toBe(true);
}

function requestState(request: unknown): unknown {
  if (!isRecord(request) || !isRecord(request.params)) {
    return undefined;
  }
  return request.params.state;
}

function requestSessionPath(request: unknown): unknown {
  if (!isRecord(request) || !isRecord(request.params)) {
    return undefined;
  }
  return request.params.agent_session_path;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

const ompControlDirectory = `/tmp/trailboss-omp-control-${process.geteuid?.()}`;
const ompControlEndpoint = `${ompControlDirectory}/${process.pid}.sock`;
const restartRequest = {
  operation: "restart-if-idle",
  pid: process.pid,
  pane_id: "test:p1",
  session_path: "/custom/sessions/exact-conversation.jsonl",
};

type RestartTestState = {
  idle: boolean;
  pending: boolean;
  editor: string;
  sessionPath: string;
  jobs: {
    running: unknown[];
    recent: unknown[];
    delivery: { queued: number; delivering: boolean };
  } | null;
  shutdowns: number;
};

async function startRestartHarness() {
  await startRecordingServer("omp-restart-control");
  const harness = createExtensionHarness();
  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(harness.pi);
  const state: RestartTestState = {
    idle: true,
    pending: false,
    editor: "",
    sessionPath: restartRequest.session_path,
    jobs: { running: [], recent: [], delivery: { queued: 0, delivering: false } },
    shutdowns: 0,
  };
  const context = {
    hasUI: true,
    isIdle: () => state.idle,
    hasPendingMessages: () => state.pending,
    ui: { getEditorText: () => state.editor },
    getAsyncJobSnapshot: () => state.jobs,
    sessionManager: {
      getSessionFile: () => state.sessionPath,
      getSessionId: () => "exact-conversation",
    },
    shutdown: () => { state.shutdowns += 1; },
  };
  const ready = Promise.withResolvers<void>();
  net.createServer = (...args) => {
    const endpointServer = Reflect.apply(createServer, net, args);
    endpointServer.once("listening", ready.resolve);
    endpointServer.once("error", ready.reject);
    return endpointServer;
  };
  try {
    harness.handlers.get("session_start")?.({}, context);
  } finally {
    net.createServer = createServer;
  }
  await ready.promise;
  return { ...harness, state, context };
}

function controlRequest(frame = `${JSON.stringify(restartRequest)}\n`): Promise<unknown> {
  const { promise, resolve, reject } = Promise.withResolvers<unknown>();
  const client = net.createConnection(ompControlEndpoint);
  let response = "";
  client.setEncoding("utf8");
  client.setTimeout(1500, () => client.destroy(new Error("control-response-timeout")));
  client.on("error", reject);
  client.on("connect", () => client.write(frame));
  client.on("data", (chunk) => { response += chunk; });
  client.on("end", () => {
    try {
      resolve(JSON.parse(response));
    } catch (error) {
      reject(error);
    }
  });
  return promise;
}

test("OMP restart endpoint is private and admits only one graceful shutdown request", async () => {
  const { state } = await startRestartHarness();
  expect(lstatSync(ompControlDirectory).mode & 0o777).toBe(0o700);
  expect(lstatSync(ompControlEndpoint).mode & 0o777).toBe(0o600);
  expect(await controlRequest()).toEqual({ status: "accepted", reason: "shutdown-requested" });
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "restart-already-accepted" });
  expect(state.shutdowns).toBe(1);
});

const restartGuards: Array<{
  name: string;
  change: (state: RestartTestState) => void;
  reason: string;
}> = [
  { name: "active runtime", change: (state) => { state.idle = false; }, reason: "runtime-not-idle" },
  { name: "pending messages", change: (state) => { state.pending = true; }, reason: "pending-messages" },
  { name: "draft including whitespace", change: (state) => { state.editor = " "; }, reason: "editor-not-empty" },
  { name: "running async job", change: (state) => { state.jobs!.running.push({ id: "worker" }); }, reason: "async-jobs-running" },
  { name: "queued async delivery", change: (state) => { state.jobs!.delivery.queued = 1; }, reason: "async-delivery-pending" },
  { name: "in-flight async delivery", change: (state) => { state.jobs!.delivery.delivering = true; }, reason: "async-delivery-pending" },
  { name: "session changed without report event", change: (state) => { state.sessionPath = "/custom/other.jsonl"; }, reason: "session-mismatch" },
];

for (const guard of restartGuards) {
  test(`OMP restart skips ${guard.name} using live context`, async () => {
    const { state } = await startRestartHarness();
    guard.change(state);
    expect(await controlRequest()).toEqual({ status: "skipped", reason: guard.reason });
    expect(state.shutdowns).toBe(0);
  });
}

for (const capability of ["isIdle", "hasPendingMessages", "getAsyncJobSnapshot", "shutdown"]) {
  test(`OMP restart fails closed without public ${capability}`, async () => {
    const { context, state } = await startRestartHarness();
    Reflect.deleteProperty(context, capability);
    expect(await controlRequest()).toEqual({ status: "skipped", reason: "runtime-capability-unavailable" });
    expect(state.shutdowns).toBe(0);
  });
}

test("OMP restart fails closed without editor or with unreadable async state", async () => {
  const { context, state } = await startRestartHarness();
  Reflect.deleteProperty(context.ui, "getEditorText");
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "runtime-capability-unavailable" });
  context.ui.getEditorText = () => "";
  context.getAsyncJobSnapshot = () => { throw new Error("unavailable"); };
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "runtime-state-unavailable" });
  expect(state.shutdowns).toBe(0);
});

for (const snapshot of ["null", "historical"]) {
  test(`OMP restart permits ${snapshot} async snapshot with no active work`, async () => {
    const { state } = await startRestartHarness();
    if (snapshot === "null") state.jobs = null;
    else state.jobs!.recent.push({ id: "completed-worker" });
    expect(await controlRequest()).toEqual({ status: "accepted", reason: "shutdown-requested" });
    expect(state.shutdowns).toBe(1);
  });
}

test("OMP restart fails closed for malformed async snapshot", async () => {
  const { context, state } = await startRestartHarness();
  Reflect.set(context, "getAsyncJobSnapshot", () => undefined);
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "async-state-unavailable" });
  expect(state.shutdowns).toBe(0);
});

test("OMP restart rejects malformed, oversized, and foreign requests without shutdown", async () => {
  const { state } = await startRestartHarness();
  const cases = [
    ["not-json\n", "invalid-request"],
    [`${"x".repeat(8193)}\n`, "request-too-large"],
    [`${JSON.stringify(restartRequest)}\n{}\n`, "invalid-frame"],
    [`${JSON.stringify({ ...restartRequest, pid: process.pid + 1 })}\n`, "pid-mismatch"],
    [`${JSON.stringify({ ...restartRequest, pane_id: "foreign" })}\n`, "pane-mismatch"],
    [`${JSON.stringify({ ...restartRequest, session_path: "/stale.jsonl" })}\n`, "session-mismatch"],
    [`${JSON.stringify({ ...restartRequest, session_path: "relative.jsonl" })}\n`, "invalid-request"],
  ];
  for (const [frame, reason] of cases) {
    expect(await controlRequest(frame)).toEqual({ status: "skipped", reason });
  }
  expect(state.shutdowns).toBe(0);
});

test("OMP restart preserves waiting, explicit blockers, active and retry guards", async () => {
  const { state, handlers, eventHandlers, context } = await startRestartHarness();
  handlers.get("tool_approval_requested")?.({}, context);
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "reporter-not-idle" });
  handlers.get("tool_approval_resolved")?.({}, context);
  eventHandlers.get("herdr:blocked")?.({ active: true }, context);
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "reporter-not-idle" });
  eventHandlers.get("herdr:blocked")?.({ active: false }, context);
  handlers.get("agent_start")?.({}, context);
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "reporter-not-idle" });
  handlers.get("agent_end")?.({ messages: [{ role: "assistant", stopReason: "error", errorMessage: "503 unavailable" }] }, context);
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "reporter-not-idle" });
  expect(state.shutdowns).toBe(0);
});

test("OMP restart captures switched context and flushes acceptance across synchronous shutdown", async () => {
  const { handlers, context, state } = await startRestartHarness();
  const switched = {
    ...context,
    ui: { getEditorText: () => "new draft" },
    shutdown: () => {
      state.shutdowns += 1;
      handlers.get("session_shutdown")?.({}, undefined);
    },
  };
  handlers.get("session_switch")?.({ reason: "resume" }, switched);
  expect(await controlRequest()).toEqual({ status: "skipped", reason: "editor-not-empty" });
  switched.ui.getEditorText = () => "";
  expect(await controlRequest()).toEqual({ status: "accepted", reason: "shutdown-requested" });
  expect(state.shutdowns).toBe(1);
  expect(existsSync(ompControlEndpoint)).toBe(false);
});

test("OMP shutdown closes unfinished control clients and removes its endpoint", async () => {
  const { handlers } = await startRestartHarness();
  const client = net.createConnection(ompControlEndpoint);
  const connected = Promise.withResolvers<void>();
  client.once("connect", connected.resolve);
  client.once("error", connected.reject);
  await connected.promise;
  client.write('{"operation":');
  const closed = Promise.withResolvers<void>();
  client.once("close", closed.resolve);
  handlers.get("session_shutdown")?.({}, undefined);
  await closed.promise;
  expect(existsSync(ompControlEndpoint)).toBe(false);
});

test("OMP extension reload never replaces or removes another live endpoint", async () => {
  const { context } = await startRestartHarness();
  const second = createExtensionHarness();
  const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
  install(second.pi);
  const before = lstatSync(ompControlEndpoint);
  second.handlers.get("session_start")?.({}, context);
  second.handlers.get("session_shutdown")?.({}, undefined);
  expect(lstatSync(ompControlEndpoint).ino).toBe(before.ino);
  expect(await controlRequest()).toEqual({ status: "accepted", reason: "shutdown-requested" });
});

test("OMP reporter refuses a symlink endpoint without following or unlinking it", async () => {
  const { handlers, context } = await startRestartHarness();
  handlers.get("session_shutdown")?.({}, undefined);
  symlinkSync(`/tmp/trailboss-absent-target-${process.pid}`, ompControlEndpoint);
  try {
    const replacement = createExtensionHarness();
    const { default: install } = await importFresh("./omp/trailboss-agent-state.ts");
    install(replacement.pi);
    replacement.handlers.get("session_start")?.({}, context);
    replacement.handlers.get("session_shutdown")?.({}, undefined);
    expect(lstatSync(ompControlEndpoint).isSymbolicLink()).toBe(true);
  } finally {
    unlinkSync(ompControlEndpoint);
  }
});
