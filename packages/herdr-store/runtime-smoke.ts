import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { z } from 'zod'
import { Declaration } from '../../plugins/agent-tree/project'
import { createHerdrStore } from './index'

// Disposable real-core smoke for the carried-over store and producer transport.
const binary = resolve(z.string().min(1).parse(process.argv[2]))
const root = await mkdtemp('/tmp/hs-')
const session = `sdk-${process.pid}`
const socket = join(root, 'herdr-fork/sessions', session, 'herdr.sock')
const env = { ...process.env }
for (const key of Object.keys(env)) if (key.startsWith('HERDR_')) delete env[key]
for (const key of [
  'HOME',
  'XDG_CONFIG_HOME',
  'XDG_STATE_HOME',
  'XDG_DATA_HOME',
  'XDG_CACHE_HOME',
  'XDG_RUNTIME_DIR',
])
  env[key] = root
Object.assign(env, { HERDR_STARTUP_CWD: root, SHELL: '/bin/sh' })
const server = Bun.spawn([binary, '--session', session, 'server'], {
  env,
  cwd: root,
  stdin: 'ignore',
  stdout: 'ignore',
  stderr: 'inherit',
})
const store = createHerdrStore(socket, () => {})
let producer: Bun.Subprocess<'pipe', 'pipe', 'inherit'> | undefined
let reading: Promise<void> | undefined
let latest: z.infer<typeof Declaration> | undefined
let ack: string | undefined
const Result = z.object({
  herdr: z.literal('action_result'),
  id: z.string(),
  ok: z.boolean(),
  error: z.string().optional(),
})
async function until(label: string, predicate: () => boolean) {
  const deadline = Date.now() + 10000
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error(`${label} timed out`)
    await Bun.sleep(10)
  }
  console.log(`PASS ${label}`)
}
try {
  await Promise.race([
    store.ready(),
    Bun.sleep(10000).then(() => {
      throw new Error('store readiness timed out')
    }),
  ])
  const Workspace = z.object({
    workspace: z.object({ workspace_id: z.string() }),
    tab: z.object({ tab_id: z.string() }),
  })
  const created = Workspace.parse(
    await store.commands.request('workspace.create', {
      cwd: root,
      label: 'Runtime smoke',
      focus: true,
    }),
  )
  const Tab = z.object({ tab: z.object({ tab_id: z.string() }) })
  const target = Tab.parse(
    await store.commands.request('tab.create', {
      workspace_id: created.workspace.workspace_id,
      cwd: root,
      label: 'Selected tab',
      focus: false,
    }),
  )
  producer = Bun.spawn(['bun', 'agent_tree.ts', '--watch'], {
    cwd: resolve(import.meta.dir, '../../plugins/agent-tree'),
    env: { ...env, HERDR_SOCKET_PATH: socket, HERDR_PLUGIN_STATE_DIR: join(root, 'plugin-state') },
    stdin: 'pipe',
    stdout: 'pipe',
    stderr: 'inherit',
  })
  const stream = producer.stdout
  if (!(stream instanceof ReadableStream)) throw new Error('producer stdout is not piped')
  reading = (async () => {
    const decoder = new TextDecoder()
    let pending = ''
    for await (const chunk of stream) {
      pending += decoder.decode(chunk, { stream: true })
      while (true) {
        const end = pending.indexOf('\n')
        if (end === -1) break
        const value: unknown = JSON.parse(pending.slice(0, end))
        pending = pending.slice(end + 1)
        const result = Result.safeParse(value)
        if (result.success) {
          assert.ok(result.data.ok, result.data.error ?? 'producer action failed')
          ack = result.data.id
        } else latest = Declaration.parse(value)
      }
    }
  })()
  await until(
    'first real widget stream contains named workspace and tab',
    () =>
      latest?.zones[0].tree.children.some((row) =>
        row.children.some((child) => child.type === 'label' && child.text === 'Runtime smoke'),
      ) === true &&
      latest.zones[0].tree.children.some((row) => row.context?.tab_id === target.tab.tab_id),
  )
  const stdin = producer.stdin
  if (typeof stdin === 'number' || stdin === undefined)
    throw new Error('producer stdin is not piped')
  stdin.write(
    `${JSON.stringify({ herdr: 'action', id: 'focus-real', action_id: 'focus-tab', context: { tab_id: target.tab.tab_id, workspace_id: created.workspace.workspace_id } })}\n`,
  )
  await stdin.flush()
  await until(
    'retained stdin envelope resolves through commands.focusTab',
    () =>
      ack === 'focus-real' &&
      store.entities.toArray.some(
        (row) => row.kind === 'tab' && row.tabId === target.tab.tab_id && row.focused,
      ),
  )
} finally {
  producer?.kill()
  if (producer) await producer.exited
  if (reading) await reading
  await store.close()
  server.kill()
  await server.exited
  await rm(root, { recursive: true, force: true })
}
