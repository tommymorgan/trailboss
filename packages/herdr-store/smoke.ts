import assert from 'node:assert/strict'
import { z } from 'zod'
import { createHerdrStore } from './index'

const socketPath = process.argv[2]
if (!socketPath || process.argv[3] !== '--allow-mutations') {
  throw new Error('Use a disposable server: bun smoke.ts /absolute/socket --allow-mutations')
}
const store = createHerdrStore(socketPath)
const WorkspaceResult = z.object({ workspace: z.object({ workspace_id: z.string() }) })
const TabResult = z.object({ tab: z.object({ tab_id: z.string() }) })
let workspaceId: string | undefined
let batches = 0
const subscription = store.tree.subscribeChanges(() => {
  batches++
})
async function waitFor(label: string, predicate: () => boolean) {
  const deadline = Date.now() + 5000
  while (!predicate()) {
    if (Date.now() >= deadline) throw new Error(`${label} timed out`)
    await Bun.sleep(20)
  }
  console.log(`PASS ${label}`, JSON.stringify(store.tree.toArray))
}
try {
  await store.ready()
  console.log('PASS initial snapshot', JSON.stringify(store.entities.toArray))
  const created = WorkspaceResult.parse(
    await store.commands.request('workspace.create', {
      cwd: '/var/tmp',
      label: 'TanStack trial',
      focus: true,
    }),
  )
  workspaceId = created.workspace.workspace_id
  await waitFor('workspace creation joins its initial tab', () =>
    store.tree.toArray.some((r) => r.workspaceId === workspaceId),
  )
  const tab = TabResult.parse(
    await store.commands.request('tab.create', {
      workspace_id: workspaceId,
      cwd: '/var/tmp',
      label: 'Adapter tab',
      focus: false,
    }),
  ).tab
  await waitFor('tab creation', () =>
    store.tree.toArray.some((r) => r.tabId === tab.tab_id && r.tab === 'Adapter tab'),
  )
  await store.commands.request('tab.rename', { tab_id: tab.tab_id, label: 'Renamed live' })
  await waitFor('tab rename', () =>
    store.tree.toArray.some((r) => r.tabId === tab.tab_id && r.tab === 'Renamed live'),
  )
  await store.commands.request('workspace.rename', {
    workspace_id: workspaceId,
    label: 'Workspace renamed',
  })
  await waitFor('workspace rename propagates through join', () => {
    const rows = store.tree.toArray.filter((r) => r.workspaceId === workspaceId)
    return rows.length === 2 && rows.every((r) => r.workspace === 'Workspace renamed')
  })
  await store.commands.request('tab.focus', { tab_id: tab.tab_id })
  await waitFor('focus transition', () =>
    store.tree.toArray.some((r) => r.tabId === tab.tab_id && r.focused),
  )
  const pane = store.entities.toArray.find((row) => row.kind === 'pane' && row.tabId === tab.tab_id)
  assert.ok(pane)
  const paneId = pane.id.slice(5)
  for (const state of ['working', 'blocked', 'idle']) {
    await store.commands.request('pane.report_agent', {
      pane_id: paneId,
      source: 'herdr-store-smoke',
      agent: 'claude',
      state,
    })
    await waitFor(
      `agent status ${state}`,
      () =>
        store.entities.get(pane.id)?.agentStatus === state &&
        store.tree.toArray.some((row) => row.tabId === tab.tab_id && row.agentStatus === state),
    )
  }
  await store.refresh()
  await Bun.sleep(150)
  const before = { ...store.stats }
  const batchesBefore = batches
  await store.refresh()
  await Bun.sleep(20)
  assert.equal(store.stats.commits, before.commits)
  assert.equal(batches, batchesBefore)
  console.log('PASS unchanged snapshot produces no collection writes or query notifications')
  const reconnect = createHerdrStore(socketPath)
  try {
    await reconnect.ready()
    assert.deepEqual(
      reconnect.tree.toArray.map(({ $collectionId, ...row }) => row),
      store.tree.toArray.map(({ $collectionId, ...row }) => row),
    )
    const retainedRows = reconnect.entities.toArray
    assert.ok(retainedRows.length > 0)
    reconnect.stop()
    assert.deepEqual(reconnect.entities.toArray, retainedRows)
    console.log('PASS stopping transport retains populated authoritative rows')
  } finally {
    await reconnect.close()
  }
  console.log('PASS new adapter resnapshots authoritative state')
  await store.commands.request('tab.close', { tab_id: tab.tab_id })
  await waitFor('tab deletion', () => !store.tree.toArray.some((r) => r.tabId === tab.tab_id))
  await store.commands.request('workspace.close', { workspace_id: workspaceId })
  await waitFor(
    'workspace deletion removes dependent tabs and panes',
    () => !store.entities.toArray.some((r) => r.workspaceId === workspaceId),
  )
  workspaceId = undefined
  console.log('PASS live smoke', JSON.stringify({ queryBatches: batches, ...store.stats }))
} finally {
  if (workspaceId) await store.commands.request('workspace.close', { workspace_id: workspaceId })
  subscription.unsubscribe()
  await store.close()
}
