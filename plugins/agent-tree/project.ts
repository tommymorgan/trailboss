import type { EntityRow } from '@herdr/store'
import { z } from 'zod'
import { Composition } from './widgets'

export const Context = z.object({
  workspace_id: z.string().optional(),
  tab_id: z.string().optional(),
  pane_id: z.string().optional(),
})
export const Role = z.enum([
  'label',
  'muted',
  'rule',
  'status-working',
  'status-blocked',
  'status-waiting',
  'status-done',
  'status-idle',
])
export const Label = z.object({ type: z.literal('label'), text: z.string(), role: Role })
export const Leader = z.object({ type: z.literal('leader') })
export const Row = z.object({
  type: z.literal('row'),
  children: z.array(z.union([Label, Leader])),
  action: z.enum(['open-workspace', 'focus-tab']).optional(),
  context: Context.optional(),
})
export type Row = z.infer<typeof Row>
export const Column = z.object({ type: z.literal('column'), children: z.array(Row) })
export const Declaration = z.object({
  herdr: z.literal('widget_tree'),
  version: z.literal(1),
  zones: z.array(z.object({ zone: z.literal('list'), tree: Column })),
  composition: Composition.optional(),
})
export type Declaration = z.infer<typeof Declaration>

export function icon(status: string) {
  if (status === 'working') return '●'
  if (status === 'blocked') return '◉'
  if (status === 'waiting') return '◌'
  return '○'
}
export function statusRole(status: string): z.infer<typeof Role> {
  const role = Role.safeParse(`status-${status}`)
  return role.success ? role.data : 'label'
}
function label(text: string, role: z.infer<typeof Role>): z.infer<typeof Label> {
  return { type: 'label', text, role }
}

// The producer specifies structure and semantic roles, never terminal widths.
export function projectTree(entities: EntityRow[], collapsed: Set<string>): z.infer<typeof Column> {
  const workspaces = entities
    .filter((row) => row.kind === 'workspace')
    .sort((a, b) => a.number - b.number)
  const tabs = entities.filter((row) => row.kind === 'tab').sort((a, b) => a.number - b.number)
  const rows: Row[] = []
  for (const workspace of workspaces) {
    const children = tabs.filter((tab) => tab.workspaceId === workspace.workspaceId)
    const hidden = collapsed.has(workspace.workspaceId)
    const statuses = children.length
      ? children.map((tab) => tab.agentStatus)
      : [workspace.agentStatus]
    const active = statuses.filter(
      (status) => status === 'working' || status === 'blocked' || status === 'waiting',
    )
    const header: Row['children'] = [label(workspace.label, 'label'), { type: 'leader' }]
    if (active.length) {
      for (const status of active) header.push(label(`${icon(status)} `, statusRole(status)))
    } else header.push(label('○ ', 'rule'))
    if (hidden && children.length) header.push(label(`${children.length} hidden`, 'muted'))
    rows.push({
      type: 'row',
      children: header,
      action: 'open-workspace',
      context: { workspace_id: workspace.workspaceId },
    })
    if (hidden) continue
    for (const tab of children) {
      if (tab.tabId === null) throw new Error('Tab entity has no tab identity')
      rows.push({
        type: 'row',
        action: 'focus-tab',
        context: { workspace_id: workspace.workspaceId, tab_id: tab.tabId },
        children: [
          label('  ', 'label'),
          label(icon(tab.agentStatus), statusRole(tab.agentStatus)),
          label(` ${tab.label}`, 'label'),
        ],
      })
    }
  }
  return { type: 'column', children: rows }
}

export function declareTree(
  hostname: string,
  entities: EntityRow[],
  collapsed: Set<string>,
): Declaration {
  const tree = projectTree(entities, collapsed)
  tree.children.unshift({ type: 'row', children: [label(` ${hostname} · herdr`, 'muted')] })
  return Declaration.parse({
    herdr: 'widget_tree',
    version: 1,
    zones: [{ zone: 'list', tree }],
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
  })
}
