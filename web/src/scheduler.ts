// GenScheduler — viewport-aware generation scheduling (S8).
//
// Before S8 the Editor opened a single WS and fired every function request at
// once on open; the backend processed that socket serially (effective
// concurrency = 1, in arrival order). This module replaces that with:
//   1. viewport-proximity ordering — the function you are looking at generates
//      first (treats the "懵/慢" perception, 反馈#1);
//   2. bounded concurrency — a small pool of parallel sockets (each backend
//      socket is serial, so N sockets = N concurrent LLM calls);
//   3. scroll re-prioritization — scrolling re-orders the *not-yet-dispatched*
//      queue so the new viewport's functions jump ahead.
//
// Per §0 (2026-06-15): the WHOLE file always generates — there is no large-file
// viewport gating (that would reintroduce the deprecated "视口激活", CONTEXT
// 激活单元). Scrolling only changes ORDER, never whether a function generates.
//
// Frontend-only (touches no backend / WS protocol). The pure pieces
// (`viewportDistance`, `PendingQueue`) carry the orderable logic and are
// deterministically unit-tested (scripts/scheduler-check.ts, B2); the parallel
// sockets + scroll wiring are browser-verified (A2).

import type { GenFrame } from './ghostTypes'

export type FnId = string

/** Visible line span of the editor (1-indexed, inclusive). */
export interface ViewportLines {
  fromLine: number
  toLine: number
}

/**
 * Generation priority of a function by how far its definition line sits from the
 * viewport: 0 when in view, otherwise the line-distance to the nearest viewport
 * edge. Lower sorts sooner. Pure — unit-tested.
 */
export function viewportDistance(fnStartLine: number, view: ViewportLines): number {
  if (fnStartLine >= view.fromLine && fnStartLine <= view.toLine) return 0
  return fnStartLine < view.fromLine ? view.fromLine - fnStartLine : fnStartLine - view.toLine
}

/**
 * The not-yet-dispatched function ids, kept ordered by viewport distance. The
 * transport pulls the nearest pending id via `shift`. Pure (no sockets) so the
 * ordering / re-prioritization / retry behavior is deterministically testable.
 */
export class PendingQueue {
  private pending: FnId[] = []

  /** Seed the queue from a fresh roster, ordered by ascending distance. */
  set(ids: FnId[], dist: Map<FnId, number>): void {
    this.pending = [...ids]
    this.sortBy(dist)
  }

  /** Re-order the remaining pending ids (scroll → new distances). */
  reprioritize(dist: Map<FnId, number>): void {
    this.sortBy(dist)
  }

  private sortBy(dist: Map<FnId, number>): void {
    // Stable sort by distance; unknown ids sort last (treated as far away).
    const d = (id: FnId) => dist.get(id) ?? Number.MAX_SAFE_INTEGER
    this.pending = this.pending
      .map((id, i) => ({ id, i }))
      .sort((a, b) => d(a.id) - d(b.id) || a.i - b.i)
      .map((x) => x.id)
  }

  /** Take the highest-priority pending id, or undefined when empty. */
  shift(): FnId | undefined {
    return this.pending.shift()
  }

  /** Put an id at the front (retry of a visible failure jumps the queue). */
  pushFront(id: FnId): void {
    if (!this.pending.includes(id)) this.pending.unshift(id)
  }

  has(id: FnId): boolean {
    return this.pending.includes(id)
  }

  /** Current order — for assertions and telemetry. */
  get order(): readonly FnId[] {
    return this.pending
  }

  get size(): number {
    return this.pending.length
  }
}

export interface SchedulerOptions {
  wsUrl: string
  /** Build the JSON request payload for one function (must set reqId = fnId). */
  buildRequest: (fnId: FnId) => unknown
  /** Route an inbound frame (capsule/line/done/error) for rendering + progress. */
  onFrame: (frame: GenFrame) => void
  /** Concurrent sockets; clamped to [1, 5]. Default 4. */
  poolSize?: number
}

/** One parallel worker: a socket plus the function it is currently handling. */
interface Worker {
  sock: WebSocket
  current: FnId | null
}

/**
 * Drives bounded-concurrency generation over a pool of WebSockets. Each worker
 * pulls the nearest pending function, sends one request, and waits for that
 * function's terminal frame (done/error) before pulling the next — so in-flight
 * requests never exceed the pool size.
 */
export class GenScheduler {
  private readonly queue = new PendingQueue()
  private readonly workers: Worker[] = []
  private readonly poolSize: number
  private readonly opts: SchedulerOptions
  private stopped = false

  constructor(opts: SchedulerOptions) {
    this.opts = opts
    this.poolSize = Math.min(5, Math.max(1, opts.poolSize ?? 4))
  }

  /** Begin scheduling: seed the queue, then open workers (≤ pool, ≤ work). */
  start(ids: FnId[], dist: Map<FnId, number>): void {
    this.stopped = false
    this.queue.set(ids, dist)
    const want = Math.min(this.poolSize, ids.length)
    for (let i = 0; i < want; i++) this.spawnWorker()
  }

  /** Scroll: re-order the pending queue so the new viewport leads. */
  reprioritize(dist: Map<FnId, number>): void {
    this.queue.reprioritize(dist)
  }

  /** Retry one failed function (S7.6): jump it to the front, wake a worker. */
  retry(id: FnId): void {
    if (this.stopped) return
    this.queue.pushFront(id)
    // Feed an idle worker, or open one if the pool has shrunk to nothing.
    const idle = this.workers.find((w) => w.current === null && w.sock.readyState === WebSocket.OPEN)
    if (idle) this.pump(idle)
    else if (this.workers.every((w) => w.sock.readyState > WebSocket.OPEN)) this.spawnWorker()
  }

  /** Tear down every socket and clear the queue (file switch / unmount). */
  stop(): void {
    this.stopped = true
    this.queue.set([], new Map())
    for (const w of this.workers) {
      w.sock.onopen = null
      w.sock.onmessage = null
      w.sock.onerror = null
      w.sock.onclose = null
      try {
        w.sock.close()
      } catch {
        /* already closing */
      }
    }
    this.workers.length = 0
  }

  private spawnWorker(): void {
    const sock = new WebSocket(this.opts.wsUrl)
    const worker: Worker = { sock, current: null }
    this.workers.push(worker)
    sock.onopen = () => {
      if (this.stopped) return
      this.pump(worker)
    }
    sock.onmessage = (ev) => {
      if (this.stopped) return
      let frame: GenFrame
      try {
        frame = JSON.parse(ev.data as string) as GenFrame
      } catch {
        return
      }
      this.opts.onFrame(frame)
      // done/error are terminal for the function — free this worker, pull next.
      if (frame.kind === 'done' || frame.kind === 'error') {
        if (worker.current === frame.reqId) worker.current = null
        this.pump(worker)
      }
    }
  }

  /** If the worker is free and has an open socket, send the next pending fn. */
  private pump(worker: Worker): void {
    if (this.stopped || worker.current !== null) return
    if (worker.sock.readyState !== WebSocket.OPEN) return
    const next = this.queue.shift()
    if (next === undefined) return
    worker.current = next
    console.debug('[sched] dispatch', next, 'pending=', this.queue.size)
    worker.sock.send(JSON.stringify(this.opts.buildRequest(next)))
  }
}
