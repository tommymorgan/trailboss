import { expect, test } from 'bun:test'
import { hostname } from 'node:os'
import { snapshotRows } from '@trailboss/store'
import { declareTree } from './project'
import { declareStock } from './stock'
import type { Widget } from './widgets'

function visibleWidgets(widget: Widget, list?: Widget): Widget[] {
  switch (widget.type) {
    case 'row':
    case 'column':
      return [widget, ...widget.children.flatMap((child) => visibleWidgets(child, list))]
    case 'flex':
      return visibleWidgets(widget.child, list)
    case 'zone': {
      const child = widget.zone === 'list' && list ? list : widget.fallback
      return child ? visibleWidgets(child, list) : []
    }
    case 'when':
      return []
    default:
      return [widget]
  }
}

test('stock sidebar keeps one session title with either the plugin list or its fallback', () => {
  const stock = declareStock([])
  if (!stock.composition) throw new Error('Missing stock composition')
  const plugin = declareTree(hostname(), [], new Set()).zones[0].tree
  for (const list of [undefined, plugin]) {
    const widgets = visibleWidgets(stock.composition.tree, list)
    const titles = widgets.filter((widget) => widget.type === 'label' && widget.role === 'title')
    expect(titles).toHaveLength(1)
  }
})

test('stock fallback and live plugin retain scoped menus without visible creation controls', () => {
  const entities = snapshotRows({
    type: 'session_snapshot',
    snapshot: {
      workspaces: [
        { workspace_id: 'w', label: 'Space', number: 1, focused: true, agent_status: 'idle' },
      ],
      tabs: [
        {
          workspace_id: 'w',
          tab_id: 't',
          label: 'Tab',
          number: 1,
          focused: true,
          agent_status: 'idle',
        },
      ],
      panes: [],
    },
  })
  const stock = declareStock(entities)
  const plugin = declareTree('host', entities, new Set())
  if (!stock.composition) throw new Error('Missing composition')
  expect(stock.composition.priority).toBeGreaterThan(plugin.composition?.priority ?? 0)
  for (const list of [undefined, plugin.zones[0].tree]) {
    const widgets = visibleWidgets(stock.composition.tree, list)
    const rows = widgets.filter((item) => item.type === 'row')
    expect(rows.filter((item) => item.action?.startsWith('host:new'))).toHaveLength(0)
    expect(
      rows.find((item) => item.menu?.type === 'use' && item.menu.name === 'sidebar.space-menu:w')
        ?.menu,
    ).toEqual({ type: 'use', name: 'sidebar.space-menu:w' })
    expect(
      rows.find((item) => item.menu?.type === 'use' && item.menu.name === 'sidebar.session-menu')
        ?.children,
    ).toEqual([{ type: 'label', text: '', role: 'muted' }])
    expect(
      rows.find((item) => item.menu?.type === 'use' && item.menu.name === 'sidebar.tab-menu:t')
        ?.menu,
    ).toEqual({ type: 'use', name: 'sidebar.tab-menu:t' })
  }
})

test('space summaries deduplicate statuses in first-tab order in both projections', () => {
  const cases = [
    [[], ['status-idle']],
    [['idle', 'idle'], ['status-idle']],
    [['unknown', 'idle', 'unknown'], ['status-idle']],
    [
      ['working', 'working', 'blocked', 'waiting', 'blocked', 'done', 'unknown', 'idle'],
      ['status-working', 'status-blocked', 'status-waiting', 'status-done', 'status-idle'],
    ],
    [
      ['unknown', 'working', 'unknown', 'working'],
      ['status-idle', 'status-working'],
    ],
  ] as const
  for (const [statuses, expected] of cases) {
    const entities = snapshotRows({
      type: 'session_snapshot',
      snapshot: {
        workspaces: [
          { workspace_id: 'w', label: 'Space', number: 1, focused: true, agent_status: 'idle' },
        ],
        tabs: statuses.map((status, index) => ({
          workspace_id: 'w',
          tab_id: `t${index}`,
          label: 'Tab',
          number: index + 1,
          focused: index === 0,
          agent_status: status,
        })),
        panes: [],
      },
    })
    const stock = declareStock(entities)
    if (!stock.composition) throw new Error('Missing composition')
    const plugin = declareTree('host', entities, new Set()).zones[0].tree
    for (const list of [undefined, plugin]) {
      const widgets = visibleWidgets(stock.composition.tree, list)
      const header = widgets.find(
        (item) =>
          item.type === 'row' &&
          item.children.some((child) => child.type === 'label' && child.role === 'section'),
      )
      if (!header || header.type !== 'row') throw new Error('Missing space header')
      expect(
        header.children
          .filter((child) => child.type === 'label' && child.role.startsWith('status-'))
          .map((child) => (child.type === 'label' ? child.role : '')),
      ).toEqual([...expected])
      for (const [index, status] of statuses.entries()) {
        const tab = widgets.find(
          (item) =>
            item.type === 'row' &&
            item.context?.tab_id === `t${index}` &&
            item.children[0]?.type === 'label' &&
            item.children[0].role.startsWith('status-'),
        )
        if (!tab || tab.type !== 'row') throw new Error('Missing tab indicator')
        expect(tab.children[0]).toMatchObject({
          text: status === 'idle' || status === 'unknown' ? '○' : '●',
          role: `status-${status === 'unknown' ? 'idle' : status}`,
        })
      }
    }
    const collapsed = declareTree('host', entities, new Set(['w'])).zones[0].tree
    const header = collapsed.children.find((item) => item.context?.workspace_id === 'w')!
    expect(
      header.children
        .filter((child) => child.type === 'label' && child.role.startsWith('status-'))
        .map((child) => (child.type === 'label' ? child.role : '')),
    ).toEqual([...expected])
  }
})
