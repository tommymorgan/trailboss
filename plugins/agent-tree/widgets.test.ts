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
