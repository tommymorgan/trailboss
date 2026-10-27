import { z } from 'zod'

const Reply = z.object({ result: z.unknown().optional(), error: z.unknown().optional() })
const Event = z.object({ event: z.string(), data: z.unknown() })
export const Subscription = z.object({ type: z.string(), pane_id: z.string().optional() })
export type SubscriptionSpec = z.infer<typeof Subscription>
export interface SubscriptionHandle {
  ready: Promise<void>
  close(): void
}
export interface TrailbossTransport {
  request(method: string, params?: object): Promise<unknown>
  close(): void
  subscribe(
    specs: SubscriptionSpec[],
    onEvent: () => void,
    onError: (error: unknown) => void,
  ): SubscriptionHandle
}

export const topologySubscriptions: SubscriptionSpec[] = [
  'workspace.created',
  'workspace.updated',
  'workspace.metadata_updated',
  'workspace.renamed',
  'workspace.moved',
  'workspace.reordered',
  'workspace.closed',
  'workspace.focused',
  'worktree.created',
  'worktree.opened',
  'worktree.removed',
  'tab.created',
  'tab.closed',
  'tab.focused',
  'tab.renamed',
  'tab.moved',
  'tab.transferred',
  'pane.created',
  'pane.closed',
  'pane.updated',
  'pane.focused',
  'pane.moved',
  'pane.exited',
  'pane.agent_detected',
].map((type) => ({ type }))

// Bun's node:net socket layer delivers some readable data on a ~100ms slow
// path, which dominates every request and event; use Bun.connect directly.
interface LineSocket {
  write(payload: string): void
  end(): void
  terminate(): void
}

function connectLineSocket(
  socketPath: string,
  onLine: (line: string) => void,
  onEnd: (error?: unknown) => void,
): LineSocket {
  let buffer = Buffer.alloc(0)
  let socket: Bun.Socket | undefined
  let ended = false
  const pending: string[] = []
  const end = (error?: unknown) => {
    if (ended) return
    ended = true
    onEnd(error)
  }
  Bun.connect({
    unix: socketPath,
    socket: {
      open(connected) {
        socket = connected
        while (pending.length) {
          const out = pending.shift()
          if (out !== undefined) socket?.write(out)
        }
      },
      data(_socket, chunk) {
        buffer = Buffer.concat([buffer, chunk])
        while (true) {
          const index = buffer.indexOf(0x0a)
          if (index === -1) break
          const line = buffer.subarray(0, index).toString('utf8')
          buffer = buffer.subarray(index + 1)
          if (line.length) onLine(line)
        }
      },
      close() {
        end()
      },
      error(_socket, error) {
        end(error)
      },
    },
  }).catch(end)
  return {
    write(payload) {
      if (ended) return
      if (socket) socket.write(payload)
      else pending.push(payload)
    },
    end() {
      socket?.end()
    },
    terminate() {
      socket?.terminate()
    },
  }
}

export function trailbossTransport(socketPath: string): TrailbossTransport {
  const open: Set<LineSocket> = new Set()
  let closed = false
  function request(method: string, params: object = {}): Promise<unknown> {
    if (closed) return Promise.reject(new Error('Transport closed'))
    const { promise, resolve, reject } = Promise.withResolvers<unknown>()
    const payload = `${JSON.stringify({ id: 'trailboss-store', method, params })}\n`
    let settled = false
    const socket = connectLineSocket(
      socketPath,
      (line) => {
        try {
          const reply = Reply.parse(JSON.parse(line))
          if (reply.error !== undefined) throw new Error(JSON.stringify(reply.error))
          finish(undefined, reply.result)
        } catch (error) {
          finish(error)
        }
      },
      (error) => finish(error ?? new Error(`${method}: server disconnected`)),
    )
    open.add(socket)
    const finish = (error?: unknown, result?: unknown) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      open.delete(socket)
      socket.end()
      if (error !== undefined) reject(error)
      else resolve(result)
    }
    const timer = setTimeout(() => {
      socket.terminate()
      finish(new Error(`${method} timed out`))
    }, 5000)
    socket.write(payload)
    return promise
  }

  function subscribe(
    specs: SubscriptionSpec[],
    onEvent: () => void,
    onError: (error: unknown) => void,
  ): SubscriptionHandle {
    const socket = connectLineSocket(
      socketPath,
      (line) => {
        try {
          const value: unknown = JSON.parse(line)
          if (first) {
            z.object({ result: z.object({ type: z.literal('subscription_started') }) }).parse(value)
            first = false
            clearTimeout(timer)
            acknowledge()
          } else {
            Event.parse(value)
            onEvent()
          }
        } catch (error) {
          fail(error)
        }
      },
      () => fail(new Error('Trailboss event stream disconnected')),
    )
    open.add(socket)
    let first = true
    let stopped = false
    const {
      promise: ready,
      resolve: acknowledge,
      reject: rejectReady,
    } = Promise.withResolvers<void>()
    // A dropped connection may fail before the consumer starts awaiting readiness.
    void ready.catch(() => {})
    let failed = false
    function fail(error: unknown) {
      if (stopped || failed) return
      failed = true
      rejectReady(error)
      onError(error)
      socket.terminate()
    }
    const timer = setTimeout(() => fail(new Error('subscription timed out')), 5000)
    socket.write(
      `${JSON.stringify({
        id: 'trailboss-store-events',
        method: 'events.subscribe',
        params: { subscriptions: z.array(Subscription).parse(specs) },
      })}\n`,
    )
    return {
      ready,
      close() {
        stopped = true
        rejectReady(new Error('Subscription closed'))
        clearTimeout(timer)
        open.delete(socket)
        socket.terminate()
      },
    }
  }
  return {
    request,
    subscribe,
    close() {
      closed = true
      for (const socket of open) socket.terminate()
      open.clear()
    },
  }
}
