import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { z } from 'zod'
import { Declaration } from '../../plugins/agent-tree/project'
import { createTrailbossStore } from './index'

// Disposable real-core smoke for the carried-over store and producer transport.
const binary = resolve(z.string().min(1).parse(process.argv[2]))
const root = await mkdtemp('/tmp/hs-')
const session = `sdk-${process.pid}`
const socket = join(root, 'trailboss/sessions', session, 'trailboss.sock')
const env = { ...process.env }
for (const key of Object.keys(env))
  if (key.startsWith('TRAILBOSS_') || key.startsWith('HERDR_')) delete env[key]
for (const key of [
  'HOME',
  'XDG_CONFIG_HOME',
  'XDG_STATE_HOME',
  'XDG_DATA_HOME',
  'XDG_CACHE_HOME',
  'XDG_RUNTIME_DIR',
])
  env[key] = root
Object.assign(env, { TRAILBOSS_STARTUP_CWD: root, SHELL: '/bin/sh' })
const server = Bun.spawn([binary, '--session', session, 'server'], {
  env,
  cwd: root,
  stdin: 'ignore',
  stdout: 'ignore',
  stderr: 'inherit',
})
let store = createTrailbossStore(socket, () => {})
let observer = createTrailbossStore(socket, () => {})
let producer: Bun.Subprocess<'pipe', 'pipe', 'inherit'> | undefined
let reading: Promise<void> | undefined
let latest: z.infer<typeof Declaration> | undefined
let ack: string | undefined
const Result = z.object({
  herdr: z.literal('action_result'),
  id: z.string(),
  ok: z.boolean(),
  error: z.string().optional(),
})
async function until(label: string, predicate: () => boolean) {
  const deadline = Date.now() + 10000
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error(`${label} timed out`)
    await Bun.sleep(10)
  }
  console.log(`PASS ${label}`)
}
try {
  await Promise.race([
    store.ready(),
    Bun.sleep(10000).then(() => {
      throw new Error('store readiness timed out')
    }),
  ])
  const Workspace = z.object({
    workspace: z.object({ workspace_id: z.string() }),
    tab: z.object({ tab_id: z.string() }),
  })
  const created = Workspace.parse(
    await store.commands.request('workspace.create', {
      cwd: root,
      label: 'Runtime smoke',
      focus: true,
    }),
  )
  const Tab = z.object({ tab: z.object({ tab_id: z.string() }) })
  const target = Tab.parse(
    await store.commands.request('tab.create', {
      workspace_id: created.workspace.workspace_id,
      cwd: root,
      label: 'Selected tab',
      focus: false,
    }),
  )
  await observer.ready()
  producer = Bun.spawn([process.execPath, 'agent_tree.ts', '--watch'], {
    cwd: resolve(import.meta.dir, '../../plugins/agent-tree'),
    env: {
      ...env,
      HERDR_SOCKET_PATH: socket,
      HERDR_PLUGIN_STATE_DIR: join(root, 'plugin-state'),
    },
    stdin: 'pipe',
    stdout: 'pipe',
    stderr: 'inherit',
  })
  const stream = producer.stdout
  if (!(stream instanceof ReadableStream)) throw new Error('producer stdout is not piped')
  reading = (async () => {
    const decoder = new TextDecoder()
    let pending = ''
    for await (const chunk of stream) {
      pending += decoder.decode(chunk, { stream: true })
      while (true) {
        const end = pending.indexOf('\n')
        if (end === -1) break
        const value: unknown = JSON.parse(pending.slice(0, end))
        pending = pending.slice(end + 1)
        const result = Result.safeParse(value)
        if (result.success) {
          assert.ok(result.data.ok, result.data.error ?? 'producer action failed')
          ack = result.data.id
        } else latest = Declaration.parse(value)
      }
    }
  })()
  await until(
    'first real widget stream contains named workspace and tab',
    () =>
      latest?.zones[0].tree.children.some((row) =>
        row.children.some((child) => child.type === 'label' && child.text === 'Runtime smoke'),
      ) === true &&
      latest.zones[0].tree.children.some((row) => row.context?.tab_id === target.tab.tab_id),
  )
  const stdin = producer.stdin
  if (typeof stdin === 'number' || stdin === undefined)
    throw new Error('producer stdin is not piped')
  stdin.write(
    `${JSON.stringify({ herdr: 'action', id: 'focus-real', action_id: 'focus-tab', context: { tab_id: target.tab.tab_id, workspace_id: created.workspace.workspace_id } })}\n`,
  )
  await stdin.flush()
  await until(
    'retained stdin envelope resolves through commands.focusTab',
    () =>
      ack === 'focus-real' &&
      store.entities.toArray.some(
        (row) => row.kind === 'tab' && row.tabId === target.tab.tab_id && row.focused,
      ),
  )
  const other = Workspace.parse(
    await store.commands.request('workspace.create', { cwd: root, label: 'Other space', focus: false }),
  )
  const workspaceId = created.workspace.workspace_id
  const otherId = other.workspace.workspace_id
  const tabIds = [created.tab.tab_id, target.tab.tab_id]
  const workspaceIds = z.object({ snapshot: z.object({
    workspaces: z.array(z.object({ workspace_id: z.string() })),
  }) }).parse(await store.commands.request('session.snapshot')).snapshot.workspaces.map((row) => row.workspace_id)
  const orderedIds = (client: typeof store, kind: 'workspace' | 'tab') =>
    client.entities.toArray
      .filter((row) => row.kind === kind && (kind === 'workspace' || row.workspaceId === workspaceId))
      .sort((a, b) => a.position - b.position)
      .map((row) => kind === 'workspace' ? row.workspaceId : row.tabId)
  const declarationIds = (action: string) =>
    latest?.zones[0].tree.children
      .filter((row) => row.action === action && (action === 'open-workspace' || row.context?.workspace_id === workspaceId))
      .map((row) => action === 'open-workspace' ? row.context?.workspace_id : row.context?.tab_id)
  const equals = (actual: unknown, expected: unknown) => JSON.stringify(actual) === JSON.stringify(expected)
  await until('two independent stores see the initial topology', () =>
    [store, observer].every((client) => equals(orderedIds(client, 'workspace'), workspaceIds) && equals(orderedIds(client, 'tab'), tabIds)),
  )
  const identities = () => store.entities.toArray
    .map(({ id, kind, workspaceId, tabId, paneId, number }) => ({ id, kind, workspaceId, tabId, paneId, tabNumber: kind === 'tab' ? number : null }))
    .sort((a, b) => a.id.localeCompare(b.id))
  const initialIdentities = identities()
  const movedWorkspaces = [otherId, ...workspaceIds.filter((id) => id !== otherId)]
  const movedTabs = [target.tab.tab_id, created.tab.tab_id]
  await store.commands.request('workspace.move', { workspace_id: otherId, insert_index: 0 })
  await store.commands.request('tab.move', { tab_id: created.tab.tab_id, insert_index: 2 })
  await until('workspace and tab moves reach both stores and the live producer without refresh', () =>
    [store, observer].every((client) => equals(orderedIds(client, 'workspace'), movedWorkspaces) && equals(orderedIds(client, 'tab'), movedTabs)) &&
    equals(declarationIds('open-workspace'), movedWorkspaces) && equals(declarationIds('focus-tab'), movedTabs),
  )
  assert.deepEqual(identities(), initialIdentities, 'moves preserve public entity identities')
  await observer.commands.request('workspace.move_block', { workspace_ids: workspaceIds, before_workspace_id: null })
  await until('atomic block reorder reaches both stores and producer', () =>
    [store, observer].every((client) => equals(orderedIds(client, 'workspace'), workspaceIds)) && equals(declarationIds('open-workspace'), workspaceIds),
  )
  const authoritative = z.object({ snapshot: z.object({
    workspaces: z.array(z.object({ workspace_id: z.string() })),
    tabs: z.array(z.object({ workspace_id: z.string(), tab_id: z.string() })),
  }) }).parse(await store.commands.request('session.snapshot'))
  assert.deepEqual(authoritative.snapshot.workspaces.map((row) => row.workspace_id), workspaceIds)
  assert.deepEqual(authoritative.snapshot.tabs.filter((row) => row.workspace_id === workspaceId).map((row) => row.tab_id), movedTabs)
  await store.close()
  await observer.close()
  store = createTrailbossStore(socket, () => {})
  observer = createTrailbossStore(socket, () => {})
  await Promise.all([store.ready(), observer.ready()])
  assert.deepEqual(orderedIds(store, 'workspace'), workspaceIds)
  assert.deepEqual(orderedIds(observer, 'workspace'), workspaceIds)
  assert.deepEqual(orderedIds(store, 'tab'), movedTabs)
  assert.deepEqual(orderedIds(observer, 'tab'), movedTabs)
  assert.deepEqual(identities(), initialIdentities, 'reattach retains public entity identities')
  console.log('PASS both stores detach and reattach to the same Core with shared order and stable IDs')
} finally {
  producer?.kill()
  if (producer) await producer.exited
  if (reading) await reading
  await store.close()
  await observer.close()
  server.kill()
  await server.exited
  await rm(root, { recursive: true, force: true })
}
