import { afterEach, describe, expect, test } from 'bun:test'
import { unlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { z } from 'zod'
import { createHerdrStore, snapshotRows } from './index'

let sockets = 0
function socketPath() {
  return join(tmpdir(), `herdr-store-test-${process.pid}-${++sockets}.sock`)
}

function snapshotOf(
  over: {
    workspaces?: Array<Record<string, unknown>>
    tabs?: Array<Record<string, unknown>>
    panes?: Array<Record<string, unknown>>
  } = {},
) {
  return {
    type: 'session_snapshot',
    snapshot: {
      workspaces: over.workspaces ?? [
        { workspace_id: 'w1', label: 'Alpha', number: 1, focused: true, agent_status: 'unknown' },
      ],
      tabs: over.tabs ?? [
        {
          workspace_id: 'w1',
          tab_id: 'w1:t1',
          label: 'Build',
          number: 1,
          focused: true,
          agent_status: 'unknown',
        },
      ],
      panes: over.panes ?? [
        {
          workspace_id: 'w1',
          tab_id: 'w1:t1',
          pane_id: 'w1:p1',
          focused: true,
          agent_status: 'unknown',
        },
      ],
    },
  }
}

// Only the surface the fake server needs from a connected socket.
interface SubSocket {
  write(chunk: string): void
  end(): void
}

const Message = z.object({
  id: z.string().optional(),
  method: z.string(),
  params: z.record(z.string(), z.unknown()).default({}),
})
const SubscribeParams = z.object({
  subscriptions: z.array(z.object({ type: z.string(), pane_id: z.string().optional() })),
})

interface FakeServer {
  requests: Array<{ method: string }>
  paneSubscriptions: Set<string>
  closedPaneSubscriptions: Set<string>
  snapshot: unknown
  push(event: string): void
  stop(): void
}

function startServer(path: string, snapshot: unknown): FakeServer {
  const requests: FakeServer['requests'] = []
  const connections = new Map<SubSocket, { buffer: string; panes: Set<string> }>()
  const fake: FakeServer = {
    requests,
    paneSubscriptions: new Set(),
    closedPaneSubscriptions: new Set(),
    snapshot,
    push(event: string) {
      for (const [socket, state] of connections) {
        if (state.panes.size) socket.write(`${JSON.stringify({ event, data: {} })}\n`)
      }
    },
    stop() {
      for (const socket of connections.keys()) socket.end()
      server.stop(true)
      try {
        unlinkSync(path)
      } catch {
        /* already gone */
      }
    },
  }
  const server = Bun.listen({
    unix: path,
    socket: {
      data(socket, chunk) {
        const state = connections.get(socket) ?? { buffer: '', panes: new Set<string>() }
        connections.set(socket, state)
        state.buffer += chunk.toString()
        for (;;) {
          const newline = state.buffer.indexOf('\n')
          if (newline < 0) return
          const line = state.buffer.slice(0, newline)
          state.buffer = state.buffer.slice(newline + 1)
          if (!line.trim()) continue
          const message = Message.parse(JSON.parse(line))
          requests.push({ method: message.method })
          if (message.method === 'events.subscribe') {
            const params = SubscribeParams.parse(message.params)
            for (const spec of params.subscriptions) {
              if (spec.pane_id) {
                state.panes.add(spec.pane_id)
                fake.paneSubscriptions.add(spec.pane_id)
              }
            }
            socket.write(
              `${JSON.stringify({ id: message.id ?? 'herdr-store-events', result: { type: 'subscription_started' } })}\n`,
            )
          } else {
            socket.write(
              `${JSON.stringify({ id: message.id ?? 'herdr-store', result: fake.snapshot })}\n`,
            )
            socket.end()
          }
        }
      },
      close(socket) {
        const state = connections.get(socket)
        connections.delete(socket)
        for (const id of state?.panes ?? []) fake.closedPaneSubscriptions.add(id)
      },
    },
  })
  return fake
}

// The store owns its coalesce (20 ms) and reconnect-retry (250 ms) timers with no
// injection seam, and real socket I/O drives every transition, so fake timers
// cannot advance this integration suite. Polling here waits for observable facts
// (server requests, collection stats, error flags), never for a fixed duration.
async function until(description: string, predicate: () => boolean, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (predicate()) return
    await Bun.sleep(10)
  }
  throw new Error(`timed out waiting for ${description}`)
}

const openStores: Array<{ close(): Promise<void> }> = []
afterEach(async () => {
  while (openStores.length) await openStores.pop()?.close()
})

describe('snapshotRows mapping', () => {
  test('maps wire workspaces, tabs, and panes onto normalized entities', () => {
    const rows = snapshotRows(snapshotOf())
    expect(rows.map((row) => row.id)).toEqual(['workspace:w1', 'tab:w1:t1', 'pane:w1:p1'])
    expect(rows[2]).toMatchObject({
      kind: 'pane',
      workspaceId: 'w1',
      tabId: 'w1:t1',
      agent: null,
      cwd: null,
      label: '',
    })
  })

  test('rejects malformed snapshots instead of guessing', () => {
    expect(() =>
      snapshotRows({
        type: 'session_snapshot',
        snapshot: { workspaces: [{ workspace_id: 'w1' }] },
      }),
    ).toThrow()
    expect(() => snapshotRows({ type: 'something_else' })).toThrow()
  })
})

describe('store synchronization against a scripted server', () => {
  test('seeds from the first snapshot, then writes nothing for an unchanged snapshot', async () => {
    const path = socketPath()
    const server = startServer(path, snapshotOf())
    const store = createHerdrStore(path, () => {})
    openStores.push(store)
    await store.ready()

    expect([...store.entities.toArray.map((row) => row.id)].sort()).toEqual([
      'pane:w1:p1',
      'tab:w1:t1',
      'workspace:w1',
    ])
    expect(store.tree.toArray.map((row) => `${row.workspace}/${row.tab}`)).toEqual(['Alpha/Build'])
    expect(store.stats).toMatchObject({ inserts: 3, updates: 0, deletes: 0, commits: 1 })
    expect(server.paneSubscriptions).toContain('w1:p1')

    const snapshotsBefore = server.requests.filter(
      (request) => request.method === 'session.snapshot',
    ).length
    server.push('tab.renamed')
    await until(
      'event-driven refresh',
      () =>
        server.requests.filter((request) => request.method === 'session.snapshot').length >
        snapshotsBefore,
    )

    expect(store.stats).toMatchObject({ inserts: 3, updates: 0, deletes: 0, commits: 1 })
    server.stop()
  }, 10000)

  test('applies updates and deletions and closes the removed pane subscription', async () => {
    const path = socketPath()
    const server = startServer(path, snapshotOf())
    const store = createHerdrStore(path, () => {})
    openStores.push(store)
    await store.ready()

    server.snapshot = snapshotOf({
      tabs: [
        {
          workspace_id: 'w1',
          tab_id: 'w1:t1',
          label: 'Ship',
          number: 1,
          focused: true,
          agent_status: 'unknown',
        },
      ],
      panes: [],
    })
    server.push('pane.closed')
    await until('rename commit', () => store.stats.commits === 2)

    expect(store.tree.toArray.map((row) => row.tab)).toEqual(['Ship'])
    expect(store.entities.get('pane:w1:p1')).toBeUndefined()
    expect(store.stats).toMatchObject({ updates: 1, deletes: 1 })
    await until('removed pane subscription closes', () =>
      server.closedPaneSubscriptions.has('w1:p1'),
    )
    server.stop()
  }, 10000)

  test('status-only events update pane entities through the pane subscription', async () => {
    const path = socketPath()
    const server = startServer(path, snapshotOf())
    const store = createHerdrStore(path, () => {})
    openStores.push(store)
    await store.ready()

    server.snapshot = snapshotOf({
      panes: [
        {
          workspace_id: 'w1',
          tab_id: 'w1:t1',
          pane_id: 'w1:p1',
          focused: true,
          agent_status: 'working',
        },
      ],
    })
    server.push('pane.agent_status_changed')
    await until(
      'working status lands in the collection',
      () => store.entities.get('pane:w1:p1')?.agentStatus === 'working',
    )

    expect(store.stats.updates).toBe(1)
    server.stop()
  }, 10000)

  test('reconnects and reseeds after the server restarts on the same socket', async () => {
    const path = socketPath()
    const server = startServer(path, snapshotOf())
    const store = createHerdrStore(path, () => {})
    openStores.push(store)
    await store.ready()
    server.stop()

    await until('connection failure surfaces', () => store.error !== undefined)
    const restart = startServer(path, snapshotOf())
    await until(
      'reconnect reseed completes',
      () => store.error === undefined && store.stats.reconnects >= 1,
    )

    expect([...store.entities.toArray.map((row) => row.id)].sort()).toEqual([
      'pane:w1:p1',
      'tab:w1:t1',
      'workspace:w1',
    ])
    restart.stop()
  }, 15000)

  test('surfaces a missing socket through the error callback without ready state', async () => {
    let failed = false
    const store = createHerdrStore(socketPath(), () => {
      failed = true
    })
    openStores.push(store)
    // Starting the sync (not awaiting it) attempts the connection; TanStack
    // collections run their sync lazily, so nothing connects until preload.
    void store.entities.preload().catch(() => {})
    await until('connection failure surfaces', () => failed || store.error !== undefined)
    expect(store.error).toBeDefined()
  }, 10000)
})
