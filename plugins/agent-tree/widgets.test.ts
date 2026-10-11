import { expect, test } from 'bun:test'
import { label, WidgetDeclaration } from './widgets'

test('styled declarations carry named tones in every public zone without pixel data', () => {
  const declaration = {
    herdr: 'widget_tree',
    version: 1,
    zones: ['list', 'footer', 'footer_right'].map((zone) => ({
      zone,
      tree: {
        type: 'row',
        children: [
          label('label', 'label', 'bg-lime'),
          label('muted', 'muted'),
          { type: 'leader', tone: 'fg-cyan' },
          label('working', 'status-working'),
        ],
      },
    })),
  }
  expect(JSON.stringify(WidgetDeclaration.parse(declaration))).toBe(JSON.stringify(declaration))
  expect(
    WidgetDeclaration.safeParse({
      ...declaration,
      zones: [
        { zone: 'list', tree: { type: 'label', text: 'invalid', role: 'label', tone: '#ff0000' } },
      ],
    }).success,
  ).toBe(false)
})

test('nested row menus round-trip and reject non-column or retired editing nodes', () => {
  const declaration = {
    herdr: 'widget_tree',
    version: 1,
    zones: [
      {
        zone: 'footer',
        tree: {
          type: 'row',
          children: [],
          menu: {
            type: 'column',
            children: [
              {
                type: 'row',
                action: 'host:rename-space',
                context: { workspace_id: 'exact' },
                children: [{ type: 'input' }],
              },
            ],
          },
        },
      },
    ],
  }
  expect<unknown>(WidgetDeclaration.parse(declaration)).toEqual(declaration)
  const tree = declaration.zones[0].tree
  for (const menu of [
    { type: 'row', children: [] },
    { type: 'column', children: [{ type: 'tab_name' }] },
    { type: 'column', children: [{ type: 'when', condition: 'menu', child: { type: 'input' } }] },
  ]) {
    expect(
      WidgetDeclaration.safeParse({
        ...declaration,
        zones: [{ zone: 'footer', tree: { ...tree, menu } }],
      }).success,
    ).toBe(false)
  }
})
