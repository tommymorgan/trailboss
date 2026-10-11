import type { EntityRow } from '@trailboss/store'
import { z } from 'zod'
import { Composition, Fragment, Use, WidgetColumn } from './widgets'

export const Context = z.object({
  workspace_id: z.string().optional(),
  tab_id: z.string().optional(),
  pane_id: z.string().optional(),
})
export const Role = z.enum([
  'label',
  'section',
  'title',
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
  action: z.enum(['open-workspace', 'focus-tab', 'focus-pane']).optional(),
  context: Context.optional(),
  menu: z.union([WidgetColumn, Use]).optional(),
})
export type Row = z.infer<typeof Row>
export const Column = z.object({ type: z.literal('column'), children: z.array(Row) })
export const Declaration = z.object({
  herdr: z.literal('widget_tree'),
  version: z.literal(1),
  zones: z.array(z.object({ zone: z.enum(['list', 'footer']), tree: Column })),
  composition: Composition.optional(),
  fragments: z.array(Fragment).optional(),
})
export type Declaration = z.infer<typeof Declaration>

export function icon(status: string) {
  return status === 'idle' || status === 'unknown' ? '○' : '●'
}
export function statusRole(status: string): z.infer<typeof Role> {
  const role = Role.safeParse(`status-${status === 'unknown' ? 'idle' : status}`)
  return role.success ? role.data : 'label'
}
function label(text: string, role: z.infer<typeof Role>): z.infer<typeof Label> {
  return { type: 'label', text, role }
}

// The producer specifies structure and semantic roles, never terminal widths.
export function projectTree(entities: EntityRow[], collapsed: Set<string>): z.infer<typeof Column> {
  const workspaces = entities
    .filter((row) => row.kind === 'workspace')
    .sort((a, b) => a.position - b.position)
  const tabs = entities.filter((row) => row.kind === 'tab').sort((a, b) => a.position - b.position)
  const rows: Row[] = []
  for (const workspace of workspaces) {
    const children = tabs.filter((tab) => tab.workspaceId === workspace.workspaceId)
    const hidden = collapsed.has(workspace.workspaceId)
    const active = [
      ...new Set(
        children
          .map((tab) => tab.agentStatus)
          .filter((status) => status !== 'idle')
          .map((status) => (status === 'unknown' ? 'idle' : status)),
      ),
    ]
    if (rows.length) rows.push({ type: 'row', children: [label('', 'muted')] })
    const header: Row['children'] = [label(workspace.label, 'section'), { type: 'leader' }]
    if (active.length) {
      for (const status of active) header.push(label(`${icon(status)} `, statusRole(status)))
    } else header.push(label('○ ', 'status-idle'))
    if (hidden && children.length) header.push(label(`${children.length} hidden`, 'muted'))
    rows.push({
      type: 'row',
      children: header,
      action: 'open-workspace',
      context: { workspace_id: workspace.workspaceId },
      menu: { type: 'use', name: `sidebar.space-menu:${workspace.workspaceId}` },
    })
    if (hidden) continue
    for (const tab of children) {
      if (tab.tabId === null) throw new Error('Tab entity has no tab identity')
      rows.push({
        type: 'row',
        action: 'focus-tab',
        context: { workspace_id: workspace.workspaceId, tab_id: tab.tabId },
        menu: { type: 'use', name: `sidebar.tab-menu:${tab.tabId}` },
        children: [
          label(icon(tab.agentStatus), statusRole(tab.agentStatus)),
          label(` ${tab.label}`, 'label'),
        ],
      })
      const panes = entities
        .filter((pane) => pane.kind === 'pane' && pane.tabId === tab.tabId)
        .sort((a, b) => a.position - b.position)
      if (panes.length > 1) {
        for (const pane of panes) {
          if (pane.paneId === null) throw new Error('Pane entity has no pane identity')
          rows.push({
            type: 'row',
            action: 'focus-pane',
            context: {
              workspace_id: workspace.workspaceId,
              tab_id: tab.tabId,
              pane_id: pane.paneId,
            },
            children: [label(`  ${pane.label || pane.paneId}`, 'muted')],
          })
        }
      }
    }
  }
  return { type: 'column', children: rows }
}

function workspaceCommand(entities: EntityRow[], action: string, name: string) {
  const spaces = entities
    .filter((entity) => entity.kind === 'workspace')
    .sort((a, b) => a.position - b.position)
  return {
    name,
    priority: 0,
    tree: {
      type: 'column',
      children: spaces.length
        ? spaces.map((space) => ({
            type: 'row',
            children: [label(`${space.label} · ${space.workspaceId}`, 'label')],
            action,
            context: { workspace_id: space.workspaceId },
          }))
        : [label('No spaces available', 'muted')],
    },
  }
}

export function declareTree(
  hostname: string,
  entities: EntityRow[],
  collapsed: Set<string>,
): Declaration {
  const tree = projectTree(entities, collapsed)
  if (tree.children.length) tree.children.unshift({ type: 'row', children: [label('', 'muted')] })
  tree.children.unshift({ type: 'row', children: [label(`${hostname} · trailboss`, 'title')] })
  return Declaration.parse({
    herdr: 'widget_tree',
    version: 1,
    zones: [
      { zone: 'list', tree },
      { zone: 'footer', tree: sessionFooter() },
    ],
    fragments: [
      workspaceCommand(entities, 'expand-workspace', 'commands.expand-space'),
      workspaceCommand(entities, 'collapse-workspace', 'commands.collapse-space'),
    ],
    composition: {
      priority: 0,
      tree: {
        type: 'row',
        children: [
          {
            type: 'flex',
            weight: 1,
            child: {
              type: 'column',
              children: [
                { type: 'flex', weight: 1, child: { type: 'zone', zone: 'list' } },
                { type: 'zone', zone: 'footer' },
              ],
            },
          },
          { type: 'flex', weight: 3, child: { type: 'surface' } },
        ],
      },
    },
  })
}

export function sessionFooter(): z.infer<typeof Column> {
  return {
    type: 'column',
    children: [
      {
        type: 'row',
        children: [label('', 'muted')],
        menu: { type: 'use', name: 'sidebar.session-menu' },
      },
    ],
  }
}

/** Only a focus transition reveals a collapsed destination, never an ordinary refresh. */
export function expandFocusedDestination(
  entities: EntityRow[],
  previousTab: string | null | undefined,
  collapsed: Set<string>,
) {
  const focused = entities.find((entity) => entity.kind === 'tab' && entity.focused)
  const tabId = focused?.tabId ?? null
  const expanded =
    previousTab !== undefined &&
    tabId !== previousTab &&
    focused !== undefined &&
    collapsed.delete(focused.workspaceId)
  return { tabId, expanded }
}
