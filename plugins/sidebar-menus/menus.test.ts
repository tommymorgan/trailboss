import { expect, test } from 'bun:test'
import { snapshotRows } from '@trailboss/store'
import { WidgetDeclaration, type Widget } from '../agent-tree/widgets'
import { declareMenus } from './menus'

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

test('public fragment declarations reject duplicate or invalid names', () => {
  const declaration = declareMenus(entities)
  const fragment = declaration.fragments?.[0]
  if (!fragment) throw new Error('Missing session menu')
  for (const fragments of [
    [fragment, fragment],
    [{ ...fragment, name: '' }],
    [{ ...fragment, name: 'bad\nname' }],
    Array.from({ length: 129 }, (_, index) => ({ ...fragment, name: `fragment:${index}` })),
  ]) {
    expect(WidgetDeclaration.safeParse({ ...declaration, fragments }).success).toBe(false)
  }
  expect(
    WidgetDeclaration.safeParse({
      herdr: 'widget_tree',
      version: 1,
      zones: [],
      fragments: [{ name: 'generic', priority: 20, tree: { type: 'use', name: 'other-producer' } }],
    }).success,
  ).toBe(true)
})

function paletteRows(source = entities) {
  const tree = declareMenus(source).fragments?.find(
    (fragment) => fragment.name === 'command-palette',
  )?.tree
  if (tree?.type !== 'column') throw new Error('Missing palette')
  return tree.children
}
function paletteRow(text: string) {
  return paletteRows().find(
    (node) =>
      node.type === 'row' &&
      node.children.some((child) => child.type === 'label' && child.text === text),
  )
}
function menuChildren(node: Widget): Widget[] {
  if (node.type === 'use') {
    const tree = declareMenus(entities).fragments?.find(
      (fragment) => fragment.name === node.name,
    )?.tree
    if (!tree) throw new Error('Unresolved menu fragment')
    return menuChildren(tree)
  }
  if (node.type !== 'column') throw new Error('Expected menu column')
  return node.children.flatMap((child) => (child.type === 'use' ? menuChildren(child) : [child]))
}
test('palette name flows retain selected identities and expose public input submit/cancel', () => {
  for (const [label, action, depth] of [
    ['Rename space', 'host:rename-space', 1],
    ['Rename tab', 'host:rename-tab', 2],
    ['New space', 'host:new-space', 0],
    ['New tab', 'host:new-tab', 1],
  ] as const) {
    let target = paletteRow(label)
    for (let step = 0; step < depth; step++) {
      if (target?.type !== 'row' || target.menu?.type !== 'column')
        throw new Error('Missing target step')
      target = target.menu.children[0]
    }
    if (target?.type !== 'row' || !target.menu) throw new Error('Missing name step')
    expect(target.action).toBe(action)
    const children = menuChildren(target.menu)
    expect(children[0]).toEqual({ type: 'input' })
    expect(children.slice(1).map((node) => (node.type === 'row' ? node.action : null))).toEqual([
      'host:create',
      'host:cancel',
    ])
    if (depth) expect(target.context?.workspace_id).toBe('w')
    if (depth === 2) expect(target.context?.tab_id).toBe('t')
  }
})
test('close targets never mutate until explicit confirmation and Cancel is first', () => {
  for (const [label, depth, action] of [
    ['Close tab', 2, 'host:close-tab'],
    ['Close space', 1, 'close-space'],
  ] as const) {
    let target = paletteRow(label)
    for (let step = 0; step < depth; step++) {
      if (target?.type !== 'row' || target.menu?.type !== 'column')
        throw new Error('Missing close target')
      expect(target.action).toBeUndefined()
      target = target.menu.children[0]
    }
    if (target?.type !== 'row' || target.menu?.type !== 'column')
      throw new Error('Missing confirmation')
    expect(target.action).toBeUndefined()
    const choices = menuChildren(target.menu).filter((node) => node.type === 'row')
    expect(choices.map((node) => node.action)).toEqual(['host:cancel', action])
  }
})
test('empty and vanished targets expose no stale actions', () => {
  const empty = paletteRows([])[0]
  expect(empty?.type === 'row' && empty.menu).toEqual({
    type: 'column',
    children: [{ type: 'label', text: 'No spaces available', role: 'muted' }],
  })

  const close = paletteRows(entities.filter((entity) => entity.kind !== 'tab')).find(
    (node) =>
      node.type === 'row' &&
      node.children.some((child) => child.type === 'label' && child.text === 'Close tab'),
  )
  if (close?.type !== 'row' || close.menu?.type !== 'column')
    throw new Error('Missing close choices')
  const space = close.menu.children[0]
  expect(space?.type === 'row' && space.menu).toEqual({
    type: 'column',
    children: [{ type: 'label', text: 'No tabs in this space', role: 'muted' }],
  })
})

test('palette tab choices follow moved snapshot positions rather than public numbers or retained collection order', () => {
  const source = snapshotRows({
    type: 'session_snapshot',
    snapshot: {
      workspaces: [
        { workspace_id: 'w', label: 'Space', number: 1, focused: true, agent_status: 'idle' },
      ],
      tabs: [
        {
          workspace_id: 'w',
          tab_id: 'second',
          label: 'Second',
          number: 2,
          focused: false,
          agent_status: 'idle',
        },
        {
          workspace_id: 'w',
          tab_id: 'first',
          label: 'First',
          number: 1,
          focused: true,
          agent_status: 'idle',
        },
      ],
      panes: [],
    },
  })
  const command = paletteRows([...source].reverse())[2]
  if (command?.type !== 'row' || command.menu?.type !== 'column')
    throw new Error('Missing switch command')
  const space = command.menu.children[0]
  if (space?.type !== 'column') throw new Error('Missing grouped tab choices')

  expect(
    space.children.slice(1).map((node) => node.type === 'row' && node.context?.tab_id),
  ).toEqual(['second', 'first'])
  expect(source.filter((row) => row.kind === 'tab').map((row) => row.number)).toEqual([2, 1])
})

test('fifty-five overflowing tabs keep sidebar menus within the Core stream line budget', () => {
  const source = snapshotRows({
    type: 'session_snapshot',
    snapshot: {
      workspaces: [
        { workspace_id: 'w2', label: 'Overflow', number: 2, focused: true, agent_status: 'idle' },
      ],
      tabs: Array.from({ length: 55 }, (_, index) => ({
        workspace_id: 'w2',
        tab_id: `w2:t${index + 1}`,
        label: `Scroll test ${String(index).padStart(2, '0')}`,
        number: index + 1,
        focused: index === 0,
        agent_status: 'idle',
      })),
      panes: [],
    },
  })
  expect(Buffer.byteLength(`${JSON.stringify(declareMenus(source))}\n`)).toBeLessThanOrEqual(
    64 * 1024,
  )
})
