import { expect, test } from 'bun:test'
import { snapshotRows } from '@trailboss/store'
import { projectTree } from './project'

// Snapshot arrays specify visual order, independently of public number identity.
const entities = snapshotRows({
  type: 'session_snapshot',
  snapshot: {
    workspaces: [
      { workspace_id: 'alpha', label: 'Alpha', number: 1, focused: true, agent_status: 'working' },
      { workspace_id: 'beta', label: 'Beta', number: 2, focused: false, agent_status: 'idle' },
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
        tab_id: 'a1',
        label: 'Build',
        number: 1,
        focused: true,
        agent_status: 'working',
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

test('declares ordered flat tab sections with status clusters and spacing', () => {
  const tree = projectTree(entities, new Set())
  expect(tree).toMatchObject({
    type: 'column',
    children: [
      {
        type: 'row',
        action: 'open-workspace',
        context: { workspace_id: 'alpha' },
        children: [
          { type: 'label', text: 'Alpha', role: 'section' },
          { type: 'leader' },
          { type: 'label', text: '● ', role: 'status-working' },
          { type: 'label', text: '● ', role: 'status-waiting' },
        ],
      },
      {
        type: 'row',
        action: 'focus-tab',
        context: { tab_id: 'a1' },
        children: [
          { type: 'label', text: '●', role: 'status-working' },
          { type: 'label', text: ' Build', role: 'label' },
        ],
      },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'a2' } },
      { type: 'row', children: [{ type: 'label', text: '', role: 'muted' }] },
      {
        type: 'row',
        action: 'open-workspace',
        context: { workspace_id: 'beta' },
        children: [
          { type: 'label', text: 'Beta', role: 'section' },
          { type: 'leader' },
          { type: 'label', text: '● ', role: 'status-blocked' },
          { type: 'label', text: '● ', role: 'status-done' },
        ],
      },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'b1' } },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'b2' } },
      { type: 'row', action: 'focus-tab', context: { tab_id: 'b3' } },
    ],
  })
})

test('empty sections show one hollow idle dot rather than a stale workspace status', () => {
  const tree = projectTree(
    entities.filter((entity) => entity.kind === 'workspace'),
    new Set(),
  )
  expect(tree.children[0].children).toEqual([
    { type: 'label', text: 'Alpha', role: 'section' },
    { type: 'leader' },
    { type: 'label', text: '○ ', role: 'status-idle' },
  ])
})
