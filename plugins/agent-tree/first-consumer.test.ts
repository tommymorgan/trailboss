import { expect, test } from 'bun:test'
import { snapshotRows } from '@herdr/store'
import { projectTree } from './project'

// Unordered, nonempty session.snapshot-shaped data; expectations come from
// the accepted sidebar behavior, not from the projection implementation.
const entities = snapshotRows({
  type: 'session_snapshot',
  snapshot: {
    workspaces: [
      { workspace_id: 'beta', label: 'Beta', number: 2, focused: false, agent_status: 'idle' },
      { workspace_id: 'alpha', label: 'Alpha', number: 1, focused: true, agent_status: 'working' },
    ],
    tabs: [
      {
        workspace_id: 'beta',
        tab_id: 'b1',
        label: 'Review',
        number: 1,
        focused: false,
        agent_status: 'blocked',
      },
      {
        workspace_id: 'alpha',
        tab_id: 'a2',
        label: 'Wait',
        number: 2,
        focused: false,
        agent_status: 'waiting',
      },
      {
        workspace_id: 'alpha',
        tab_id: 'a1',
        label: 'Build',
        number: 1,
        focused: true,
        agent_status: 'working',
      },
      {
        workspace_id: 'beta',
        tab_id: 'b2',
        label: 'Done',
        number: 2,
        focused: false,
        agent_status: 'done',
      },
      {
        workspace_id: 'beta',
        tab_id: 'b3',
        label: 'Idle',
        number: 3,
        focused: false,
        agent_status: 'idle',
      },
    ],
    panes: [
      {
        workspace_id: 'alpha',
        tab_id: 'a1',
        pane_id: 'p1',
        focused: true,
        agent_status: 'working',
        agent: 'codex',
      },
    ],
  },
})

test('Port the agent tree as the first widget-tree consumer', () => {
  const tree = projectTree(entities, new Set())
  expect(Array.isArray(tree)).toBe(false)
  expect(tree).toMatchObject({
    type: 'column',
    children: [
      {
        type: 'row',
        action: 'open-workspace',
        context: { workspace_id: 'alpha' },
        children: [
          { type: 'label', text: 'Alpha', role: 'label' },
          { type: 'leader' },
          { type: 'label', text: '● ', role: 'status-working' },
          { type: 'label', text: '◌ ', role: 'status-waiting' },
        ],
      },
      {
        type: 'row',
        action: 'focus-tab',
        context: { tab_id: 'a1' },
        children: [
          { type: 'label', text: '  ', role: 'label' },
          { type: 'label', text: '●', role: 'status-working' },
          { type: 'label', text: ' Build', role: 'label' },
        ],
      },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'a2' } },
      { type: 'row', action: 'open-workspace', context: { workspace_id: 'beta' } },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'b1' } },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'b2' } },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'b3' } },
    ],
  })
})
