import { expect, test } from 'bun:test'
import { type EntityRow, snapshotRows } from '@trailboss/store'
import { actionResponseLine, parseControlLine } from './control'
import {
  Declaration,
  declareTree,
  expandFocusedDestination,
  icon,
  projectTree,
  statusRole,
} from './project'

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

test('keeps a completed command exit result visible after its dot is acknowledged', () => {
  const source = {
    type: 'session_snapshot',
    snapshot: {
      workspaces: [
        { workspace_id: 'w', label: 'Space', number: 1, focused: true, agent_status: 'idle' },
      ],
      tabs: [
        {
          workspace_id: 'w',
          tab_id: 't',
          label: 'Shell',
          number: 1,
          focused: true,
          agent_status: 'idle',
        },
      ],
      panes: [
        {
          workspace_id: 'w',
          tab_id: 't',
          pane_id: 'p',
          focused: true,
          agent_status: 'idle',
          tokens: { 'command.status': 'failure', 'command.exit_code': '7' },
        },
      ],
    },
  }
  const tab = projectTree(snapshotRows(source), new Set()).children.find(
    (row) => row.context?.tab_id === 't',
  )
  expect(tab?.children[0]).toEqual({ type: 'label', text: '○', role: 'status-idle' })
  expect(tab?.children).toContainEqual({
    type: 'label',
    text: ' · exit 7',
    role: 'muted',
  })
})

test('shows each split pane command result on its own focus target', () => {
  const pane: EntityRow = {
    ...entities[1],
    id: 'pane:p',
    kind: 'pane',
    paneId: 'p',
    label: 'Build',
    command: { status: 'failure', exitCode: 7 },
  }
  const rows = projectTree(
    [
      ...entities,
      pane,
      {
        ...pane,
        id: 'pane:q',
        paneId: 'q',
        label: 'Check',
        command: { status: 'success', exitCode: 0 },
      },
    ],
    new Set(),
  ).children
  expect(rows.find((row) => row.context?.pane_id === 'p')?.children).toContainEqual({
    type: 'label',
    text: ' · exit 7',
    role: 'muted',
  })
  expect(rows.find((row) => row.context?.pane_id === 'q')?.children).toContainEqual({
    type: 'label',
    text: ' · exit 0',
    role: 'muted',
  })
})

test('separates the session title from the first workspace section', () => {
  const rows = declareTree('junkman', entities, new Set()).zones[0].tree.children
  expect(rows[1]).toEqual({
    type: 'row',
    children: [{ type: 'label', text: '', role: 'muted' }],
  })
  expect(rows[2]).toMatchObject({
    context: { workspace_id: 'w' },
    children: [
      { type: 'label', text: 'Workspace', role: 'section' },
      { type: 'leader' },
      { type: 'label', text: '○ ', role: 'status-idle' },
    ],
  })
})
test('retained collapsed state projects a header and hidden count without a title fallback', () => {
  const tree = projectTree(entities, new Set(['w']))
  expect(tree.children).toHaveLength(1)
  expect(tree.children[0].children).toEqual([
    { type: 'label', text: 'Workspace', role: 'section' },
    { type: 'leader' },
    { type: 'label', text: '○ ', role: 'status-idle' },
    { type: 'label', text: '1 hidden', role: 'muted' },
  ])
  expect(
    Declaration.safeParse({ herdr: 'widget_tree', version: 1, zones: [{ zone: 'list', tree }] })
      .success,
  ).toBe(true)
})
test('uses solid status dots for active tabs and hollow dots for idle tabs', () => {
  expect(['working', 'blocked', 'waiting', 'idle', 'done', 'unknown'].map(icon)).toEqual([
    '●',
    '●',
    '●',
    '○',
    '●',
    '○',
  ])
  expect(['working', 'blocked', 'waiting', 'idle', 'done', 'unknown'].map(statusRole)).toEqual([
    'status-working',
    'status-blocked',
    'status-waiting',
    'status-idle',
    'status-done',
    'status-idle',
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

test('declares a real pane focus target beneath its tab', () => {
  const pane: EntityRow = {
    ...entities[1],
    id: 'pane:p',
    kind: 'pane',
    paneId: 'p',
    label: 'Pane target',
  }
  const rows = projectTree(
    [...entities, pane, { ...pane, id: 'pane:q', paneId: 'q' }],
    new Set(),
  ).children
  expect(rows.find((item) => item.context?.pane_id === 'p')).toEqual({
    type: 'row',
    action: 'focus-pane',
    context: { workspace_id: 'w', tab_id: 't', pane_id: 'p' },
    children: [{ type: 'label', text: '  Pane target', role: 'muted' }],
  })
})

test('sidebar references independently supplied scoped menus without owning their content', () => {
  const declaration = declareTree('host', entities, new Set())
  const rows = declaration.zones[0].tree.children
  expect(rows.find((item) => item.action === 'open-workspace')?.menu).toEqual({
    type: 'use',
    name: 'sidebar.space-menu:w',
  })
  expect(rows.find((item) => item.action === 'focus-tab')?.menu).toEqual({
    type: 'use',
    name: 'sidebar.tab-menu:t',
  })
  const footer = declaration.zones.find((zone) => zone.zone === 'footer')?.tree.children[0]
  expect(footer?.children).toEqual([{ type: 'label', text: '', role: 'muted' }])
  expect(footer?.menu).toEqual({ type: 'use', name: 'sidebar.session-menu' })
  expect(declaration.fragments?.map((fragment) => fragment.name)).toEqual([
    'commands.expand-space',
    'commands.collapse-space',
  ])
})

test('focus changes reveal a collapsed destination while refreshes respect manual collapse', () => {
  const collapsed = new Set(['w', 'other'])
  expect(expandFocusedDestination(entities, undefined, collapsed).expanded).toBe(false)
  expect(expandFocusedDestination(entities, 't', collapsed).expanded).toBe(false)
  expect(collapsed.has('w')).toBe(true)
  expect(expandFocusedDestination(entities, 'old-tab', collapsed).expanded).toBe(true)
  expect([...collapsed]).toEqual(['other'])
  collapsed.add('w')
  expect(expandFocusedDestination(entities, 't', collapsed).expanded).toBe(false)
  expect(collapsed.has('w')).toBe(true)
})

test('expand and collapse commands address chosen spaces without requiring focused or expanded rows', () => {
  const space = entities.find((entity) => entity.kind === 'workspace')
  if (!space) throw new Error('Missing fixture space')
  const source = [
    ...entities,
    { ...space, id: 'workspace:other', workspaceId: 'other', focused: false },
  ]
  const declaration = declareTree('host', source, new Set(['other']))
  for (const [name, action] of [
    ['commands.expand-space', 'expand-workspace'],
    ['commands.collapse-space', 'collapse-workspace'],
  ]) {
    const tree = declaration.fragments?.find((fragment) => fragment.name === name)?.tree
    if (tree?.type !== 'column') throw new Error('Missing workspace choices')
    const choices = tree.children.filter((node) => node.type === 'row')
    expect(choices.map((node) => node.context?.workspace_id)).toEqual(['w', 'other'])
    expect(choices.map((node) => node.action)).toEqual([action, action])
    expect(choices[1]?.children[0]).toEqual({
      type: 'label',
      text: 'Workspace · other',
      role: 'label',
    })
  }
  expect(source.find((entity) => entity.workspaceId === 'other')?.focused).toBe(false)
  const empty = declareTree('host', [], new Set()).fragments?.[0]?.tree
  expect(empty).toEqual({
    type: 'column',
    children: [{ type: 'label', text: 'No spaces available', role: 'muted' }],
  })
})

test('reordered snapshot declares moved tabs first while public tab numbers remain stable', () => {
  const source = {
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
  }
  const rows = snapshotRows(source)
  // Collections may enumerate retained identities rather than their latest positions.
  const tree = projectTree([...rows].reverse(), new Set())
  expect(
    tree.children.filter((row) => row.action === 'focus-tab').map((row) => row.children[1]),
  ).toEqual([
    { type: 'label', text: ' Second', role: 'label' },
    { type: 'label', text: ' First', role: 'label' },
  ])
  expect(rows.filter((row) => row.kind === 'tab').map((row) => row.number)).toEqual([2, 1])
})
