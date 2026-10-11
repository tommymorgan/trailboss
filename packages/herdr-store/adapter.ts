import {
  and,
  BasicIndex,
  createCollection,
  createLiveQueryCollection,
  eq,
  type SyncConfig,
} from '@tanstack/db'
import { z } from 'zod'
import { herdrTransport, type SubscriptionHandle, topologySubscriptions } from './transport'

// A single collection makes cross-workspace/tab/pane snapshot changes atomic.
export const Entity = z.object({
  id: z.string(),
  kind: z.enum(['workspace', 'tab', 'pane']),
  workspaceId: z.string(),
  tabId: z.string().nullable(),
  paneId: z.string().nullable(),
  label: z.string(),
  number: z.number(),
  focused: z.boolean(),
  agentStatus: z.string(),
  agent: z.string().nullable(),
  cwd: z.string().nullable(),
})
export type EntityRow = z.infer<typeof Entity>
export const TreeRow = z.object({
  id: z.string(),
  workspaceId: z.string(),
  tabId: z.string().nullable(),
  workspace: z.string(),
  tab: z.string(),
  focused: z.boolean(),
  agentStatus: z.string(),
})
export type TreeRow = z.infer<typeof TreeRow>
const Common = z.object({ focused: z.boolean(), agent_status: z.string() })
const Workspace = Common.extend({
  workspace_id: z.string(),
  label: z.string(),
  number: z.number().int().nonnegative(),
})
const Tab = Common.extend({
  workspace_id: z.string(),
  tab_id: z.string(),
  label: z.string(),
  number: z.number().int().nonnegative(),
})
const Pane = Common.extend({
  workspace_id: z.string(),
  tab_id: z.string(),
  pane_id: z.string(),
  label: z.string().nullable().optional(),
  title: z.string().nullable().optional(),
  agent: z.string().nullable().optional(),
  cwd: z.string().nullable().optional(),
})
const Snapshot = z.object({
  type: z.literal('session_snapshot'),
  snapshot: z.object({ workspaces: z.array(Workspace), tabs: z.array(Tab), panes: z.array(Pane) }),
})
export function snapshotRows(value: unknown): EntityRow[] {
  const { snapshot } = Snapshot.parse(value)
  return [
    ...snapshot.workspaces.map(
      (w): EntityRow => ({
        id: `workspace:${w.workspace_id}`,
        kind: 'workspace',
        workspaceId: w.workspace_id,
        tabId: null,
        paneId: null,
        label: w.label,
        number: w.number,
        focused: w.focused,
        agentStatus: w.agent_status,
        agent: null,
        cwd: null,
      }),
    ),
    ...snapshot.tabs.map(
      (t): EntityRow => ({
        id: `tab:${t.tab_id}`,
        kind: 'tab',
        workspaceId: t.workspace_id,
        tabId: t.tab_id,
        paneId: null,
        label: t.label,
        number: t.number,
        focused: t.focused,
        agentStatus: t.agent_status,
        agent: null,
        cwd: null,
      }),
    ),
    ...snapshot.panes.map(
      (p): EntityRow => ({
        id: `pane:${p.pane_id}`,
        kind: 'pane',
        workspaceId: p.workspace_id,
        tabId: p.tab_id,
        paneId: p.pane_id,
        label: p.label ?? p.title ?? '',
        number: 0,
        focused: p.focused,
        agentStatus: p.agent_status,
        agent: p.agent ?? null,
        cwd: p.cwd ?? null,
      }),
    ),
  ]
}

export type PerfStage =
  | 'invalidate'
  | 'snapshot-start'
  | 'snapshot-end'
  | 'commit'
  | 'reconnect'
  | 'fail'
export function createHerdrStore(
  socketPath = process.env.HERDR_SOCKET_PATH,
  onError: (error: unknown) => void = console.error,
  onPerf?: (stage: PerfStage, detail?: Record<string, number | string>) => void,
) {
  if (!socketPath) throw new Error('Provide a Herdr JSON API socket path or HERDR_SOCKET_PATH')
  const transport = herdrTransport(socketPath)
  const commands = {
    request: transport.request,
    focusTab(tabId: string) {
      return transport.request('tab.focus', { tab_id: z.string().min(1).parse(tabId) })
    },
    focusWorkspace(workspaceId: string) {
      return transport.request('workspace.focus', {
        workspace_id: z.string().min(1).parse(workspaceId),
      })
    },
    focusPane(paneId: string) {
      return transport.request('pane.focus', { pane_id: z.string().min(1).parse(paneId) })
    },
  }
  const stats = {
    snapshots: 0,
    events: 0,
    inserts: 0,
    updates: 0,
    deletes: 0,
    commits: 0,
    reconnects: 0,
  }
  let refresh: () => Promise<void> = async () => {
    throw new Error('Preload the store first')
  }
  let stop: () => void = () => {}
  let failure: unknown
  let stopped = false
  const sync: SyncConfig<EntityRow, string> = {
    rowUpdateMode: 'full',
    sync: ({ begin, write, commit, markReady }) => {
      let previous = new Map<string, EntityRow>()
      let closed = false
      let dirty = false
      let generation = 0
      let running: Promise<void> | undefined
      let timer: NodeJS.Timeout | undefined
      let retry: NodeJS.Timeout | undefined
      let topology: SubscriptionHandle | undefined
      const panes = new Map<string, SubscriptionHandle>()
      function disconnect() {
        generation++
        topology?.close()
        topology = undefined
        for (const sub of panes.values()) sub.close()
        panes.clear()
      }
      function fail(error: unknown) {
        if (closed || retry) return
        failure = error
        onPerf?.('fail', { error: String(error) })
        disconnect()
        clearTimeout(timer)
        timer = undefined
        onError(error)
        retry = setTimeout(() => {
          retry = undefined
          if (!closed) {
            stats.reconnects++
            onPerf?.('reconnect')
            void connect().catch(fail)
          }
        }, 250)
      }
      function invalidate() {
        stats.events++
        onPerf?.('invalidate')
        dirty = true
        // Events have no causal cursor and are grouped by kind: invalidate,
        // coalesce, and read authoritative state rather than replaying deltas.
        if (!timer && !closed && !retry)
          timer = setTimeout(() => {
            timer = undefined
            void refresh().catch(fail)
          }, 20)
      }
      async function drain() {
        do {
          dirty = false
          const epoch = generation
          onPerf?.('snapshot-start')
          const rows = snapshotRows(await transport.request('session.snapshot'))
          if (closed || epoch !== generation) return
          const ids = new Set(
            rows.filter((row) => row.kind === 'pane').map((row) => row.id.slice(5)),
          )
          for (const [id, sub] of panes)
            if (!ids.has(id)) {
              sub.close()
              panes.delete(id)
            }
          const added: SubscriptionHandle[] = []
          for (const id of ids)
            if (!panes.has(id)) {
              const sub = transport.subscribe(
                [{ type: 'pane.agent_status_changed', pane_id: id }],
                invalidate,
                fail,
              )
              panes.set(id, sub)
              added.push(sub)
            }
          // A new pane can change status between snapshot and subscription ACK.
          // Seed again after watchers are installed to close that race.
          if (added.length) {
            await Promise.all(added.map((sub) => sub.ready))
            if (closed || epoch !== generation) return
            dirty = true
            continue
          }
          stats.snapshots++
          const next = new Map(rows.map((row) => [row.id, row]))
          const changed = rows.filter(
            (row) => JSON.stringify(previous.get(row.id)) !== JSON.stringify(row),
          )
          const removed = [...previous.keys()].filter((id) => !next.has(id))
          onPerf?.('snapshot-end', { changed: changed.length, removed: removed.length })
          if (changed.length || removed.length) {
            begin()
            for (const row of changed) {
              const type = previous.has(row.id) ? 'update' : 'insert'
              write({ type, value: row })
              if (type === 'update') stats.updates++
              else stats.inserts++
            }
            for (const key of removed) {
              write({ type: 'delete', key })
              stats.deletes++
            }
            commit()
            stats.commits++
            onPerf?.('commit', { changed: changed.length, removed: removed.length })
          }
          previous = next
          failure = undefined
          markReady()
        } while (dirty && !closed && !retry)
      }
      refresh = () => {
        if (closed) return Promise.reject(new Error('Store closed'))
        dirty = true
        if (!running)
          running = drain().finally(() => {
            running = undefined
          })
        return running
      }
      async function connect() {
        if (closed) return
        const epoch = generation
        topology = transport.subscribe(topologySubscriptions, invalidate, fail)
        await topology.ready
        if (closed || epoch !== generation) return
        if (running) await running.catch(() => {})
        if (!closed && epoch === generation) await refresh()
      }
      stop = () => {
        closed = true
        clearTimeout(timer)
        clearTimeout(retry)
        disconnect()
      }
      void connect().catch(fail)
      return stop
    },
  }
  const entities = createCollection({
    id: `herdr:${socketPath}`,
    schema: Entity,
    getKey: (row) => row.id,
    sync,
    gcTime: Infinity,
  })
  entities.createIndex((row) => row.workspaceId, { indexType: BasicIndex })
  entities.createIndex((row) => row.tabId, { indexType: BasicIndex })
  entities.createIndex((row) => row.kind, { indexType: BasicIndex })
  const tree = createLiveQueryCollection({
    gcTime: Infinity,
    query: (q) =>
      q
        .from({ tab: entities })
        .innerJoin({ workspace: entities }, ({ tab, workspace }) =>
          eq(tab.workspaceId, workspace.workspaceId),
        )
        .where(({ tab, workspace }) => and(eq(tab.kind, 'tab'), eq(workspace.kind, 'workspace')))
        .orderBy(({ workspace }) => workspace.number)
        .orderBy(({ tab }) => tab.number)
        .select(({ tab, workspace }) => ({
          id: tab.id,
          workspaceId: workspace.workspaceId,
          tabId: tab.tabId,
          workspace: workspace.label,
          tab: tab.label,
          focused: tab.focused,
          agentStatus: tab.agentStatus,
        })),
  })
  return {
    entities,
    tree,
    stats,
    commands,
    async ready() {
      if (stopped) throw new Error('Store closed')
      await entities.preload()
      await tree.preload()
    },
    refresh() {
      return refresh()
    },
    get error() {
      return failure
    },
    // Stop transport without erasing the last authoritative rows.
    stop() {
      stopped = true
      stop()
      transport.close()
    },
    async close() {
      stopped = true
      stop()
      transport.close()
      await tree.cleanup()
      await entities.cleanup()
    },
  }
}
