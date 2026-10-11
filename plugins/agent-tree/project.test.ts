import { expect, test } from 'bun:test'
import { snapshotRows } from '@herdr/store'
import { actionResponseLine, parseControlLine } from './control'
import { Declaration, declareTree, icon, projectTree, statusRole } from './project'

const entities = snapshotRows({
  type: 'session_snapshot',
  snapshot: {
    workspaces: [
      { workspace_id: 'w', label: 'Workspace', number: 1, focused: true, agent_status: 'idle' },
    ],
    tabs: [
      {
        workspace_id: 'w',
        tab_id: 't',
        label: 'Terminal',
        number: 1,
        focused: true,
        agent_status: 'idle',
      },
    ],
    panes: [],
  },
})

test('preserves the hostname title as a muted declaration, including empty sessions', () => {
  expect(declareTree('junkman', [], new Set())).toEqual({
    herdr: 'widget_tree',
    version: 1,
    composition: {
      priority: 0,
      tree: {
        type: 'row',
        children: [
          { type: 'flex', weight: 1, child: { type: 'zone', zone: 'list' } },
          { type: 'flex', weight: 3, child: { type: 'surface' } },
        ],
      },
    },
    zones: [
      {
        zone: 'list',
        tree: {
          type: 'column',
          children: [
            { type: 'row', children: [{ type: 'label', text: ' junkman · herdr', role: 'muted' }] },
          ],
        },
      },
    ],
  })
})
test('retained collapsed state projects a header and hidden count without a title fallback', () => {
  const tree = projectTree(entities, new Set(['w']))
  expect(tree.children).toHaveLength(1)
  expect(tree.children[0].children).toEqual([
    { type: 'label', text: 'Workspace', role: 'label' },
    { type: 'leader' },
    { type: 'label', text: '○ ', role: 'rule' },
    { type: 'label', text: '1 hidden', role: 'muted' },
  ])
  expect(
    Declaration.safeParse({ herdr: 'widget_tree', version: 1, zones: [{ zone: 'list', tree }] })
      .success,
  ).toBe(true)
})
test('preserves every fork status glyph and semantic role', () => {
  expect(['working', 'blocked', 'waiting', 'idle', 'done', 'unknown'].map(icon)).toEqual([
    '●',
    '◉',
    '◌',
    '○',
    '○',
    '○',
  ])
  expect(['working', 'blocked', 'waiting', 'idle', 'done', 'unknown'].map(statusRole)).toEqual([
    'status-working',
    'status-blocked',
    'status-waiting',
    'status-idle',
    'status-done',
    'label',
  ])
})
test('requires tab identity before declaring a focus action', () => {
  expect(() =>
    projectTree(
      entities.map((entity) => (entity.kind === 'tab' ? { ...entity, tabId: null } : entity)),
      new Set(),
    ),
  ).toThrow('Tab entity has no tab identity')
})
test('retains request and result envelope identities and all context targets', () => {
  expect(
    parseControlLine(
      '{"herdr":"action","id":"r","action_id":"focus-tab","context":{"tab_id":"t","workspace_id":"w","pane_id":"p"}}',
    ),
  ).toEqual({ id: 'r', actionId: 'focus-tab', tabId: 't', workspaceId: 'w', paneId: 'p' })
  expect(JSON.parse(actionResponseLine('r', true))).toEqual({
    herdr: 'action_result',
    id: 'r',
    ok: true,
  })
  expect(JSON.parse(actionResponseLine('r', false, 'missing tab'))).toEqual({
    herdr: 'action_result',
    id: 'r',
    ok: false,
    error: 'missing tab',
  })
  for (const line of [
    '',
    '[]',
    'null',
    '{"herdr":"action_result","id":"r"}',
    '{"herdr":"action","id":"","action_id":"focus-tab"}',
    '{"herdr":"action","id":"r","action_id":"focus-tab","context":7}',
  ])
    expect(parseControlLine(line)).toBeNull()
})
