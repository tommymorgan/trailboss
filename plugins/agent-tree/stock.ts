import { hostname } from 'node:os'
import { createTrailbossStore, type EntityRow } from '@trailboss/store'
import { z } from 'zod'
import { declareTree, sessionFooter } from './project'
import { flex, label, row, type Widget, WidgetDeclaration, when } from './widgets'

export function declareStock(entities: EntityRow[]) {
  const tree = declareTree(hostname(), entities, new Set()).zones[0].tree
  const tabs = entities.filter((entity) => entity.kind === 'tab')
  const fallback: Widget = {
    type: 'column',
    children: tree.children.map((item) => {
      const tabId =
        item.context?.tab_id ??
        tabs.find((tab) => tab.workspaceId === item.context?.workspace_id)?.tabId
      return row(
        item.children,
        tabId ? 'host:focus' : undefined,
        {
          ...item.context,
          ...(tabId ? { tab_id: tabId } : {}),
        },
        item.menu,
      )
    }),
  }
  const sidebar: Widget = {
    type: 'column',
    children: [
      flex(1, { type: 'zone', zone: 'list', fallback }),
      { type: 'zone', zone: 'footer', fallback: sessionFooter() },
      { type: 'zone', zone: 'footer_right' },
      when('new_tab', {
        type: 'column',
        children: [
          row([label(' New tab ')]),
          row([label(' Name: ')]),
          row([label(' ['), { type: 'input' }, label('] ')]),
          row([label(' Type, then Enter ')]),
          row([label(' Create ')], 'host:create'),
          row([label(' Cancel ')], 'host:cancel'),
        ],
      }),
      when('rename_space', renameModal('space')),
      when('rename_tab', renameModal('tab')),
    ],
  }
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
            when('no_pane', row([label(' No pane yet — choose New tab ')])),
            flex(1, when('has_pane', { type: 'surface' })),
          ],
        }),
      ]),
    },
  })
}

function renameModal(kind: 'space' | 'tab'): Widget {
  return {
    type: 'modal',
    child: {
      type: 'column',
      children: [
        row([label(` Rename ${kind} `, 'title')]),
        { type: 'spacer', size: 1 },
        row([label(' Name: ')]),
        row([label(' ['), { type: 'input' }, label('] ')]),
        { type: 'spacer', size: 1 },
        row([label(' [ Save ] ')], 'host:create'),
        row([label(' [ Cancel ] ')], 'host:cancel'),
      ],
    },
  }
}

if (import.meta.main) {
  const socket = z.string().min(1).parse(process.env.HERDR_SOCKET_PATH)
  const store = createTrailbossStore(socket)
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
