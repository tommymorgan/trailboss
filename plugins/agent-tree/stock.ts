import { createHerdrStore, type EntityRow } from '@herdr/store'
import { z } from 'zod'
import { projectTree } from './project'
import { flex, label, row, type Widget, WidgetDeclaration, when } from './widgets'

export function declareStock(entities: EntityRow[]) {
  const tree = projectTree(entities, new Set())
  const tabs = entities.filter((entity) => entity.kind === 'tab')
  const fallback: Widget = {
    type: 'column',
    children: tree.children.map((item) => {
      const tabId =
        item.context?.tab_id ??
        tabs.find((tab) => tab.workspaceId === item.context?.workspace_id)?.tabId
      return row(item.children, tabId ? 'host:focus' : undefined, {
        ...item.context,
        ...(tabId ? { tab_id: tabId } : {}),
      })
    }),
  }
  const sidebar: Widget = {
    type: 'column',
    children: [
      row([label(' herdr · stock', 'muted')]),
      flex(1, { type: 'zone', zone: 'list', fallback }),
      { type: 'zone', zone: 'footer', fallback: row([label(' + New tab ')], 'host:new') },
      { type: 'zone', zone: 'footer_right', fallback: row([label(' Menu ')], 'host:menu') },
    ],
  }
  const focusedWorkspace = entities.find((entity) => entity.kind === 'workspace' && entity.focused)
  const tabStrip = tabs
    .filter((tab) => tab.workspaceId === focusedWorkspace?.workspaceId)
    .map((tab) =>
      row([label(` ${tab.label} `)], 'host:focus', {
        workspace_id: tab.workspaceId,
        ...(tab.tabId ? { tab_id: tab.tabId } : {}),
      }),
    )
  return WidgetDeclaration.parse({
    herdr: 'widget_tree',
    version: 1,
    zones: [],
    composition: {
      priority: 10,
      tree: row([
        flex(1, sidebar),
        flex(3, {
          type: 'column',
          children: [
            tabStrip.length ? row(tabStrip) : { type: 'spacer', size: 1 },
            when('no_pane', row([label(' No pane yet — choose New tab ')])),
            when('new_tab', {
              type: 'column',
              children: [
                row([label(' New tab ')]),
                row([label(' Name: '), { type: 'tab_name' }]),
                row([label(' Create ')], 'host:create'),
                row([label(' Cancel ')], 'host:cancel'),
              ],
            }),
            when('menu', {
              type: 'column',
              children: [
                row([label(' Session menu ')]),
                row([label(' New tab ')], 'host:new'),
                row([label(' Close menu ')], 'host:cancel'),
              ],
            }),
            flex(1, when('has_pane', { type: 'surface' })),
          ],
        }),
      ]),
    },
  })
}

if (import.meta.main) {
  const socket = z.string().min(1).parse(process.env.HERDR_SOCKET_PATH)
  const store = createHerdrStore(socket)
  let previous = ''
  function emit() {
    const next = JSON.stringify(declareStock(store.entities.toArray))
    if (next !== previous) {
      previous = next
      process.stdout.write(`${next}\n`)
    }
  }
  const subscription = store.entities.subscribeChanges(emit)
  async function close() {
    subscription.unsubscribe()
    await store.close()
  }
  for (const signal of ['SIGINT', 'SIGTERM'])
    process.on(signal, () => {
      void close().then(() => process.exit(0))
    })
  process.stdout.on('error', () => {
    void close().then(() => process.exit(1))
  })
  await store.ready()
  emit()
}
