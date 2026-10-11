import { z } from 'zod'

// Action requests and results retain the fork's stdin/stdout envelope contract.
const Id = z.string().min(1).optional()
const Request = z.object({
  herdr: z.literal('action'),
  id: z.string().min(1),
  action_id: z.string(),
  context: z.object({ tab_id: Id, workspace_id: Id, pane_id: Id }).default({}),
})
export function parseControlLine(line: string) {
  let value: unknown
  try {
    value = JSON.parse(line)
  } catch {
    return null
  }
  const parsed = Request.safeParse(value)
  if (!parsed.success) return null
  const { id, action_id, context } = parsed.data
  return {
    id,
    actionId: action_id,
    tabId: context.tab_id,
    workspaceId: context.workspace_id,
    paneId: context.pane_id,
  }
}
export function actionResponseLine(id: string, ok: boolean, error?: string): string {
  return JSON.stringify({
    herdr: 'action_result',
    id,
    ok,
    ...(ok ? {} : { error: error ?? 'action failed' }),
  })
}
