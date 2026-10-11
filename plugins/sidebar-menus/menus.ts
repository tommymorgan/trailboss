import { createTrailbossStore, type EntityRow } from '@trailboss/store'
import { z } from 'zod'
import {
  type Fragment,
  label,
  row,
  type Use,
  type Widget,
  type WidgetColumn,
  WidgetDeclaration,
} from '../agent-tree/widgets'

function nameFragment(create: boolean): z.infer<typeof Fragment> {
  return {
    name: create ? 'commands.name-new' : 'commands.name-existing',
    priority: 0,
    tree: {
      type: 'column',
      children: [
        { type: 'input' },
        row([label(create ? 'Create' : 'Save')], 'host:create'),
        row([label('Cancel')], 'host:cancel'),
      ],
    },
  }
}

function confirmationFragment(kind: 'space' | 'tab'): z.infer<typeof Fragment> {
  return {
    name: `commands.confirm-${kind}`,
    priority: 0,
    tree: {
      type: 'column',
      children: [
        row([label('Cancel')], 'host:cancel'),
        row(
          [label(`Close ${kind}`, 'danger')],
          kind === 'space' ? 'close-space' : 'host:close-tab',
        ),
      ],
    },
  }
}

function palette(entities: EntityRow[]): z.infer<typeof Fragment> {
  const spaces = entities
    .filter((entity) => entity.kind === 'workspace')
    .sort((a, b) => a.position - b.position)
  const column = (children: Widget[]): z.infer<typeof WidgetColumn> => ({
    type: 'column',
    children,
  })
  const name = (create: boolean): z.infer<typeof Use> => ({
    type: 'use',
    name: create ? 'commands.name-new' : 'commands.name-existing',
  })
  const spaceChoices = (choose: (space: EntityRow) => Widget) =>
    column(spaces.length ? spaces.map(choose) : [label('No spaces available', 'muted')])
  const tabChoices = (space: EntityRow, action: string, menu?: Parameters<typeof row>[3]) => {
    const tabs = entities
      .filter((entity) => entity.kind === 'tab' && entity.workspaceId === space.workspaceId)
      .sort((a, b) => a.position - b.position)
    return column(
      tabs.length
        ? tabs.map((tab) => {
            if (tab.tabId === null) throw new Error('Tab entity has no tab identity')
            const context = { workspace_id: space.workspaceId, tab_id: tab.tabId }
            const next = action === 'host:close-tab' ? confirmation('tab', tab) : menu
            return row(
              [label(`${action === 'host:focus' ? '  ' : ''}${tab.label}`)],
              next && action === 'host:close-tab' ? undefined : action,
              context,
              next,
            )
          })
        : [label('No tabs in this space', 'muted')],
    )
  }
  const confirmation = (kind: 'space' | 'tab', entity: EntityRow) =>
    column([
      label(`Close ${kind} “${entity.label}”?`, 'muted'),
      { type: 'use', name: `commands.confirm-${kind}` },
    ])
  return {
    name: 'command-palette',
    priority: 0,
    tree: column([
      row(
        [label('Rename space')],
        undefined,
        undefined,
        spaceChoices((space) =>
          row(
            [label(space.label)],
            'host:rename-space',
            { workspace_id: space.workspaceId },
            name(false),
          ),
        ),
      ),
      row(
        [label('Rename tab')],
        undefined,
        undefined,
        spaceChoices((space) =>
          row(
            [label(space.label)],
            undefined,
            { workspace_id: space.workspaceId },
            tabChoices(space, 'host:rename-tab', name(false)),
          ),
        ),
      ),
      row(
        [label('Switch tab')],
        undefined,
        undefined,
        spaceChoices((space) =>
          column([
            label(space.label, 'heading'),
            ...tabChoices(space, 'host:focus').children.filter((node) => node.type === 'row'),
          ]),
        ),
      ),
      row([label('Previous tab')], 'host:focus'),
      row([label('New space')], 'host:new-space', undefined, name(true)),
      row(
        [label('New tab')],
        undefined,
        undefined,
        spaceChoices((space) =>
          row(
            [label(space.label)],
            'host:new-tab',
            { workspace_id: space.workspaceId },
            name(true),
          ),
        ),
      ),
      row(
        [label('Close tab')],
        undefined,
        undefined,
        spaceChoices((space) =>
          row(
            [label(space.label)],
            undefined,
            { workspace_id: space.workspaceId },
            tabChoices(space, 'host:close-tab'),
          ),
        ),
      ),
      row(
        [label('Close space')],
        undefined,
        undefined,
        spaceChoices((space) =>
          row(
            [label(space.label)],
            undefined,
            { workspace_id: space.workspaceId },
            confirmation('space', space),
          ),
        ),
      ),
      row([label('Expand space')], undefined, undefined, {
        type: 'use',
        name: 'commands.expand-space',
      }),
      row([label('Collapse space')], undefined, undefined, {
        type: 'use',
        name: 'commands.collapse-space',
      }),
      row([label('Restart idle agents')], 'host:restart-idle-agents'),
      row([label('Detach from session')], 'host:detach'),
    ]),
  }
}

export function declareMenus(entities: EntityRow[]) {
  const fragments: z.infer<typeof Fragment>[] = [
    {
      name: 'sidebar.session-menu',
      priority: 0,
      tree: {
        type: 'column',
        children: [
          row([label('SESSION', 'muted')]),
          row([label('New space')], 'host:new-space'),
          row([label('Restart idle agents')], 'host:restart-idle-agents'),
          row([label('Detach from session')], 'host:detach'),
        ],
      },
    },
    palette(entities),
    nameFragment(true),
    nameFragment(false),
    confirmationFragment('space'),
    confirmationFragment('tab'),
    {
      name: 'sidebar.confirm-close-space',
      priority: 0,
      tree: {
        type: 'column',
        children: [
          row([label('Close this space?', 'muted')]),
          { type: 'use', name: 'commands.confirm-space' },
        ],
      },
    },
  ]
  for (const entity of entities) {
    if (entity.kind === 'workspace')
      fragments.push({
        name: `sidebar.space-menu:${entity.workspaceId}`,
        priority: 0,
        tree: {
          type: 'column',
          children: [
            row([label(`New tab in “${entity.label}”`)], 'host:new-tab', {
              workspace_id: entity.workspaceId,
            }),
            row([label(`Rename “${entity.label}”`)], 'host:rename-space', {
              workspace_id: entity.workspaceId,
            }),
            row(
              [label('Close space', 'danger')],
              undefined,
              { workspace_id: entity.workspaceId },
              {
                type: 'use',
                name: 'sidebar.confirm-close-space',
              },
            ),
          ],
        },
      })
    if (entity.kind === 'tab') {
      if (entity.tabId === null) throw new Error('Tab entity has no tab identity')
      fragments.push({
        name: `sidebar.tab-menu:${entity.tabId}`,
        priority: 0,
        tree: {
          type: 'column',
          children: [
            row([label(`Rename “${entity.label}”`)], 'host:rename-tab'),
            row([label(`Close tab “${entity.label}”`, 'danger')], 'host:close-tab'),
          ],
        },
      })
    }
  }
  return WidgetDeclaration.parse({ herdr: 'widget_tree', version: 1, zones: [], fragments })
}

export async function closeSpace(socketPath: string, workspaceId: string) {
  const store = createTrailbossStore(z.string().min(1).parse(socketPath))
  try {
    return await store.commands.request('workspace.close', {
      workspace_id: z.string().min(1).parse(workspaceId),
      close_group: false,
    })
  } finally {
    await store.close()
  }
}

if (import.meta.main && process.argv[2] === '--close-space') {
  await closeSpace(
    z.string().min(1).parse(process.env.HERDR_SOCKET_PATH),
    z.string().min(1).parse(process.env.HERDR_WORKSPACE_ID),
  )
} else if (import.meta.main) {
  const store = createTrailbossStore(z.string().min(1).parse(process.env.HERDR_SOCKET_PATH))
  let previous = ''
  function emit() {
    const next = JSON.stringify(declareMenus(store.entities.toArray))
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
