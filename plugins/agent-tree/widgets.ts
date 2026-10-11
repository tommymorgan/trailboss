import { z } from 'zod'

export const WidgetContext = z.object({
  workspace_id: z.string().optional(),
  tab_id: z.string().optional(),
  pane_id: z.string().optional(),
})
export const Tone = z.enum(['bg-lime', 'fg-cyan'])
const ZoneName = z.enum(['list', 'footer', 'footer_right'])
export const Use = z.object({
  type: z.literal('use'),
  name: z
    .string()
    .min(1)
    .refine((name) => [...name].length <= 200)
    .refine((name) =>
      [...name].every((char) => {
        const code = char.codePointAt(0) ?? 0
        return code > 31 && (code < 127 || code > 159)
      }),
    ),
})
const Label = z.object({
  type: z.literal('label'),
  text: z.string(),
  role: z.string(),
  tone: Tone.optional(),
})
const Row = z.object({
  type: z.literal('row'),
  get children() {
    return z.array(Widget)
  },
  action: z.string().optional(),
  context: WidgetContext.optional(),
  get menu() {
    return z.union([WidgetColumn, Use]).optional()
  },
})
export const WidgetColumn = z.object({
  type: z.literal('column'),
  get children() {
    return z.array(Widget)
  },
})
const List = z.object({
  type: z.literal('list'),
  get entries() {
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
  condition: z.enum(['no_pane', 'has_pane', 'new_tab', 'rename_space', 'rename_tab']),
  get child() {
    return Widget
  },
})
const Zone = z.object({
  type: z.literal('zone'),
  zone: ZoneName,
  get fallback() {
    return Widget.optional()
  },
})
const Modal = z.object({
  type: z.literal('modal'),
  get child() {
    return Widget
  },
})
export const Widget = z.discriminatedUnion('type', [
  Row,
  WidgetColumn,
  Label,
  List,
  z.object({ type: z.literal('spacer'), size: z.number().int().min(0).max(65535) }),
  Flex,
  When,
  Zone,
  z.object({ type: z.literal('leader'), tone: Tone.optional() }),
  z.object({ type: z.literal('surface') }),
  z.object({ type: z.literal('input') }),
  Use,
  Modal,
])
export type Widget = z.infer<typeof Widget>
export const Composition = z.object({ priority: z.number().int(), tree: Widget })
export const Fragment = z.object({
  name: Use.shape.name,
  priority: z.number().int(),
  tree: Widget,
})
export const WidgetDeclaration = z.object({
  herdr: z.literal('widget_tree'),
  version: z.literal(1),
  zones: z.array(z.object({ zone: ZoneName, tree: Widget })),
  composition: Composition.optional(),
  fragments: z
    .array(Fragment)
    .max(128)
    .refine(
      (fragments) => new Set(fragments.map((fragment) => fragment.name)).size === fragments.length,
    )
    .optional(),
})
export function row(
  children: Widget[],
  action?: string,
  context?: z.infer<typeof WidgetContext>,
  menu?: z.infer<typeof WidgetColumn> | z.infer<typeof Use>,
): Widget {
  return {
    type: 'row',
    children,
    ...(action ? { action } : {}),
    ...(context ? { context } : {}),
    ...(menu ? { menu } : {}),
  }
}
export function label(text: string, role = 'label', tone?: z.infer<typeof Tone>): Widget {
  return { type: 'label', text, role, ...(tone ? { tone } : {}) }
}
export function flex(weight: number, child: Widget): Widget {
  return { type: 'flex', weight, child }
}
export function when(condition: z.infer<typeof When>['condition'], child: Widget): Widget {
  return { type: 'when', condition, child }
}
