import { z } from 'zod'

export const WidgetContext = z.object({
  workspace_id: z.string().optional(),
  tab_id: z.string().optional(),
  pane_id: z.string().optional(),
})
const Label = z.object({ type: z.literal('label'), text: z.string(), role: z.string() })
const Row = z.object({
  type: z.literal('row'),
  get children() {
    return z.array(Widget)
  },
  action: z.string().optional(),
  context: WidgetContext.optional(),
})
const Column = z.object({
  type: z.literal('column'),
  get children() {
    return z.array(Widget)
  },
})
const Flex = z.object({
  type: z.literal('flex'),
  weight: z.number().int().positive(),
  get child() {
    return Widget
  },
})
const When = z.object({
  type: z.literal('when'),
  condition: z.enum(['no_pane', 'has_pane', 'new_tab', 'menu']),
  get child() {
    return Widget
  },
})
const Zone = z.object({
  type: z.literal('zone'),
  zone: z.literal('list'),
  get fallback() {
    return Widget.optional()
  },
})
export const Widget = z.discriminatedUnion('type', [
  Row,
  Column,
  Label,
  Flex,
  When,
  Zone,
  z.object({ type: z.literal('leader') }),
  z.object({ type: z.literal('surface') }),
  z.object({ type: z.literal('tab_name') }),
])
export type Widget = z.infer<typeof Widget>
export const Composition = z.object({ priority: z.number().int(), tree: Widget })
export const WidgetDeclaration = z.object({
  herdr: z.literal('widget_tree'),
  version: z.literal(1),
  zones: z.array(z.object({ zone: z.literal('list'), tree: Widget })),
  composition: Composition.optional(),
})
export function row(
  children: Widget[],
  action?: string,
  context?: z.infer<typeof WidgetContext>,
): Widget {
  return { type: 'row', children, ...(action ? { action } : {}), ...(context ? { context } : {}) }
}
export function label(text: string, role = 'label'): Widget {
  return { type: 'label', text, role }
}
export function flex(weight: number, child: Widget): Widget {
  return { type: 'flex', weight, child }
}
export function when(
  condition: 'no_pane' | 'has_pane' | 'new_tab' | 'menu',
  child: Widget,
): Widget {
  return { type: 'when', condition, child }
}
