import { expect, test } from 'bun:test'
import { snapshotRows } from '@trailboss/store'
import { WidgetDeclaration } from '../agent-tree/widgets'
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
