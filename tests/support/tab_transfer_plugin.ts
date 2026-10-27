import { createTrailbossStore } from '../../packages/trailboss-store'
import { projectTree } from '../../plugins/agent-tree/project'

const [tabId, destination, receiptPath] = process.argv.slice(2)
if (!tabId || !destination || !receiptPath) throw new Error('Missing transfer fixture arguments')
const store = createTrailbossStore()
let receipt: object = {}
try {
  await store.ready()
  const before = store.entities.toArray
  const initialEvents = store.stats.events
  const response = await store.commands.request('tab.transfer', {
    tab_id: tabId,
    workspace_id: destination,
  })
  receipt = { before, response }
  const deadline = Date.now() + 5000
  while (
    store.entities.toArray.filter(
      (row) =>
        row.kind === 'pane' &&
        row.workspaceId === destination &&
        row.label.startsWith('Build pane'),
    ).length !== 2
  ) {
    if (Date.now() >= deadline) throw new Error('Subscribed store did not move both panes to Main')
    await Bun.sleep(10)
  }
  await Bun.write(
    receiptPath,
    JSON.stringify({
      ...receipt,
      after: store.entities.toArray,
      tree: projectTree(store.entities.toArray, new Set()),
      events: store.stats.events - initialEvents,
    }),
  )
} catch (error) {
  await Bun.write(receiptPath, JSON.stringify({ ...receipt, error: String(error) }))
  throw error
} finally {
  await store.close()
}
