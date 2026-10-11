import { createHash } from 'node:crypto'
import { watch } from 'node:fs'
import { appendFile, mkdir, readFile, rename, stat, writeFile } from 'node:fs/promises'
import { hostname as nodeHostname } from 'node:os'
import { join } from 'node:path'
import { createTrailbossStore } from '@trailboss/store'
import { z } from 'zod'
import { actionResponseLine, parseControlLine } from './control'
import { declareTree, expandFocusedDestination } from './project'

const hostname = (() => {
  try {
    return nodeHostname()
  } catch {
    return 'trailboss'
  }
})()

const Collapsed = z.array(z.string())
const socketPath = z.string().min(1).parse(process.env.HERDR_SOCKET_PATH)
const stateDir =
  process.env.HERDR_PLUGIN_STATE_DIR ??
  join(
    process.env.XDG_STATE_HOME ?? join(process.env.HOME ?? '.', '.local/state'),
    'trailboss/plugins/tommymorgan.agent-tree',
  )
// Socket identity separates named sessions even when plugin state directories are shared.
const stateName = `collapsed-${createHash('sha256').update(socketPath).digest('hex').slice(0, 16)}.json`
const statePath = join(stateDir, stateName)
await mkdir(stateDir, { recursive: true })
const sessionTag = stateName.slice('collapsed-'.length, -'.json'.length)
const perfLog = join(stateDir, 'perf', 'perf.log')
await mkdir(join(stateDir, 'perf'), { recursive: true })
async function perf(stage: string, detail: Record<string, number | string> = {}) {
  // Best-effort profiling log in a subdirectory: the collapse watcher
  // observes stateDir non-recursively, so perf writes must not retrigger it.
  try {
    await appendFile(
      perfLog,
      `${Date.now()} s=${sessionTag} stage=${stage}${Object.entries(detail)
        .map(([k, v]) => ` ${k}=${v}`)
        .join('')}\n`,
    )
  } catch {
    /* profiling is optional */
  }
}
try {
  if ((await stat(perfLog)).size > 1_000_000) await writeFile(perfLog, '')
} catch {
  /* no perf log yet */
}
const startedAt = Date.now()
async function loadCollapsed(): Promise<Set<string>> {
  try {
    return new Set(Collapsed.parse(JSON.parse(await readFile(statePath, 'utf8'))))
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') return new Set()
    throw error
  }
}
async function saveCollapsed(collapsed: Set<string>) {
  const temporary = `${statePath}.${process.pid}.tmp`
  await writeFile(temporary, JSON.stringify([...collapsed].sort()))
  await rename(temporary, statePath)
}
const mode = process.argv[2]
const store = createTrailbossStore(socketPath, undefined, (stage, detail) => {
  void perf(stage, detail)
})

async function focusTabAction(tabId: string) {
  const sentAt = Date.now()
  await store.commands.focusTab(tabId)
  await perf('action-ack', { req_ms: Date.now() - sentAt })
}

async function openWorkspaceAction(workspaceId: string) {
  const workspace = store.entities.get(`workspace:${workspaceId}`)
  if (!workspace) throw new Error(`Workspace no longer exists: ${workspaceId}`)
  if (!workspace.focused) {
    const sentAt = Date.now()
    await store.commands.focusWorkspace(workspaceId)
    await perf('action-ack', { req_ms: Date.now() - sentAt })
  } else {
    const collapsed = await loadCollapsed()
    if (collapsed.has(workspaceId)) collapsed.delete(workspaceId)
    else collapsed.add(workspaceId)
    await saveCollapsed(collapsed)
    await perf('action-collapse-written', { at_ms: Date.now() - startedAt })
  }
}

async function setWorkspaceCollapsed(workspaceId: string, hide: boolean) {
  if (
    !store.entities.toArray.some(
      (entity) => entity.kind === 'workspace' && entity.workspaceId === workspaceId,
    )
  )
    throw new Error('Workspace no longer exists')
  const collapsed = await loadCollapsed()
  if (hide) collapsed.add(workspaceId)
  else collapsed.delete(workspaceId)
  await saveCollapsed(collapsed)
  return collapsed
}

async function runAction(mode: string, tabId?: string, workspaceId?: string) {
  await store.ready()
  await perf('action-ready', { ready_ms: Date.now() - startedAt })
  if (mode === '--focus-tab') {
    await focusTabAction(
      z
        .string()
        .min(1)
        .parse(tabId ?? process.env.HERDR_TAB_ID),
    )
  } else if (mode === '--focus-pane') {
    await store.commands.focusPane(z.string().min(1).parse(process.env.HERDR_PANE_ID))
  } else if (mode === '--expand-workspace' || mode === '--collapse-workspace') {
    await setWorkspaceCollapsed(
      z
        .string()
        .min(1)
        .parse(workspaceId ?? process.env.HERDR_WORKSPACE_ID),
      mode === '--collapse-workspace',
    )
  } else {
    await openWorkspaceAction(
      z
        .string()
        .min(1)
        .parse(workspaceId ?? process.env.HERDR_WORKSPACE_ID),
    )
  }
}

if (
  [
    '--focus-tab',
    '--open-workspace',
    '--focus-pane',
    '--expand-workspace',
    '--collapse-workspace',
  ].includes(mode ?? '')
) {
  try {
    await runAction(mode)
  } finally {
    const totalMs = Date.now() - startedAt
    await perf('action-exit', { total_ms: totalMs })
    process.stderr.write(`perf action=${mode} start_ms=${startedAt} total_ms=${totalMs}\n`)
    await store.close()
  }
} else if (mode === '--watch') {
  let collapsed = await loadCollapsed()
  let previous: string | undefined
  let previousFocusedTab: string | null | undefined
  let reading: Promise<void> = Promise.resolve()
  let stopping = false
  let active = false
  let timer: NodeJS.Timeout | undefined

  function emit() {
    if (!active || stopping) return
    const declaration = declareTree(hostname, store.entities.toArray, collapsed)
    const next = JSON.stringify(declaration)
    if (next !== previous) {
      previous = next
      void perf('emit', {
        rows: declaration.zones[0].tree.children.length,
        bytes: next.length,
        at_ms: Date.now() - startedAt,
      })
      process.stdout.write(`${next}\n`)
    }
  }
  function update() {
    if (!active || stopping) return
    const entities = store.entities.toArray
    const previousTab = previousFocusedTab
    previousFocusedTab =
      entities.find((entity) => entity.kind === 'tab' && entity.focused)?.tabId ?? null
    if (previousTab === undefined || previousTab === previousFocusedTab) {
      emit()
      return
    }
    reading = reading
      .then(async () => {
        collapsed = await loadCollapsed()
        if (expandFocusedDestination(entities, previousTab, collapsed).expanded)
          await saveCollapsed(collapsed)
        emit()
      })
      .catch((error) => console.error(error))
  }
  const subscription = store.entities.subscribeChanges(update)
  // Forwarded actions from Trailboss arrive as control envelopes on stdin.
  // Bun's node:net/readline paths add a ~100ms delivery tick to stdin and
  // even poison other socket callbacks while attached; use Bun.stdin.stream.
  let stdinBuffer = Buffer.alloc(0)
  const dispatchControl = (line: string) => {
    const request = parseControlLine(line)
    if (!request || stopping) return
    reading = reading
      .then(async () => {
        const dispatched = Date.now()
        let response: string
        try {
          if (request.actionId === 'focus-tab') {
            await focusTabAction(
              z
                .string()
                .min(1)
                .parse(request.tabId ?? ''),
            )
          } else if (request.actionId === 'open-workspace') {
            await openWorkspaceAction(
              z
                .string()
                .min(1)
                .parse(request.workspaceId ?? ''),
            )
          } else if (
            request.actionId === 'expand-workspace' ||
            request.actionId === 'collapse-workspace'
          ) {
            const workspaceId = z
              .string()
              .min(1)
              .parse(request.workspaceId ?? '')
            collapsed = await setWorkspaceCollapsed(
              workspaceId,
              request.actionId === 'collapse-workspace',
            )
            emit()
          } else if (request.actionId === 'focus-pane') {
            await store.commands.focusPane(
              z
                .string()
                .min(1)
                .parse(request.paneId ?? ''),
            )
          } else {
            throw new Error(`Unknown action: ${request.actionId}`)
          }
          response = actionResponseLine(request.id, true)
        } catch (error) {
          response = actionResponseLine(
            request.id,
            false,
            error instanceof Error ? error.message : String(error),
          )
        }
        await perf('action-forward', { action: request.actionId, ms: Date.now() - dispatched })
        process.stdout.write(`${response}\n`)
      })
      .catch((error) => console.error(error))
  }
  void (async () => {
    for await (const chunk of Bun.stdin.stream()) {
      stdinBuffer = Buffer.concat([stdinBuffer, chunk])
      while (true) {
        const index = stdinBuffer.indexOf(0x0a)
        if (index === -1) break
        const line = stdinBuffer.subarray(0, index).toString('utf8')
        stdinBuffer = stdinBuffer.subarray(index + 1)
        if (line.length) dispatchControl(line)
      }
    }
  })()

  const watcher = watch(stateDir, () => {
    // Bun may coalesce an atomic rename to the temporary filename only.
    if (stopping) return
    void perf('watch-fired')
    clearTimeout(timer)
    timer = setTimeout(() => {
      reading = reading
        .then(async () => {
          collapsed = await loadCollapsed()
          void perf('collapsed-loaded', { at_ms: Date.now() - startedAt })
          emit()
        })
        .catch((error) => console.error(error))
    }, 10)
  })
  async function close() {
    if (stopping) return
    stopping = true
    clearTimeout(timer)
    watcher.close()
    subscription.unsubscribe()
    try {
      process.stdin.destroy()
    } catch {
      /* stdin may be gone */
    }
    await reading
    await store.close()
  }
  for (const signal of ['SIGINT', 'SIGTERM'])
    process.on(signal, () => {
      void close().then(() => process.exit(0))
    })
  process.stdout.on('error', (error) => {
    console.error(error)
    void close().then(() => process.exit(1))
  })
  try {
    await store.ready()
    active = true
    update()
  } catch (error) {
    await close()
    throw error
  }
} else {
  await store.close()
  throw new Error('Use --watch, --focus-tab, --focus-pane, or --open-workspace')
}
