// Thin fetch wrappers over the Rust backend's L0 endpoints (技术方案 §4).
// Requests go to /api/* and are proxied to 127.0.0.1:7878 in dev (vite.config.ts).

import type { FunctionSpan } from './parser/types.ts'
import type { LineAnnotation, QueryFrame } from './ghostTypes'
import type { CapsuleSummary } from './queryContext'

export type Lang = 'py' | 'rs' | 'other'

export interface FileNode {
  path: string
  name: string
  lang: Lang
}

/** GET /api/project/tree -> flat FileNode[] (the frontend nests it, see tree.ts). */
export async function fetchTree(): Promise<FileNode[]> {
  const res = await fetch('/api/project/tree')
  if (!res.ok) throw new Error(`/api/project/tree -> ${res.status}`)
  const data = (await res.json()) as { files: FileNode[] }
  return data.files
}

/** GET /api/file?path=<rel> -> source string (read-only). */
export async function fetchFile(path: string): Promise<string> {
  const res = await fetch(`/api/file?path=${encodeURIComponent(path)}`)
  if (!res.ok) throw new Error(`/api/file?path=${path} -> ${res.status}`)
  const data = (await res.json()) as { source: string }
  return data.source
}

/** POST /api/project/open { path } -> new canonical root (U3 single-root swap). */
export async function openFolder(path: string): Promise<string> {
  const res = await fetch('/api/project/open', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path }),
  })
  if (!res.ok) throw new Error((await res.text()) || `/api/project/open -> ${res.status}`)
  const data = (await res.json()) as { root: string }
  return data.root
}

/** POST /api/project/pick -> chosen absolute path, or null if the user cancelled
 *  the native folder dialog (opened by the local backend, U3 revision). */
export async function pickFolder(): Promise<string | null> {
  const res = await fetch('/api/project/pick', { method: 'POST' })
  if (!res.ok) throw new Error((await res.text()) || `/api/project/pick -> ${res.status}`)
  const data = (await res.json()) as { path: string | null }
  return data.path
}

/** POST /api/explain-line -> one LineAnnotation for a manually-picked non-key
 *  line (S9 手动补行). The line number must sit inside the function's range. */
export async function explainLine(req: {
  filePath: string
  fn: FunctionSpan
  lineNumber: number
}): Promise<LineAnnotation> {
  const res = await fetch('/api/explain-line', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ filePath: req.filePath, fn: req.fn, lineNumber: req.lineNumber }),
  })
  if (!res.ok) throw new Error((await res.text()) || `/api/explain-line -> ${res.status}`)
  return (await res.json()) as LineAnnotation
}

/** Callbacks for a streaming follow-up query (S10b). */
export interface QueryHandlers {
  onDelta: (text: string) => void
  onDone: () => void
  onError: (message: string) => void
}

/** Handle to an in-flight query stream; `cancel` tears the socket down silently. */
export interface QueryStream {
  cancel: () => void
}

/** Open `WS /api/query`, send one question, and stream the answer back token by
 *  token (S10a frames: delta×N → done | error). One socket per question; it is
 *  closed on the terminal frame or on `cancel` (file switch / unmount). The S10a
 *  backend treats roster/capsules/focus as optional; S10b-cap layers in the
 *  current file's roster + generated capsule summaries so the answer no longer
 *  leans on the graph's file_summary backstop alone. */
export function streamQuery(
  req: {
    filePath: string
    question: string
    roster?: string[]
    rosterSpans?: FunctionSpan[]
    capsules?: CapsuleSummary[]
  },
  h: QueryHandlers,
): QueryStream {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const sock = new WebSocket(`${proto}://${location.host}/api/query`)
  let settled = false
  const close = () => {
    try {
      sock.close()
    } catch {
      /* already closing */
    }
  }
  sock.onopen = () => {
    sock.send(
      JSON.stringify({
        reqId: 'q',
        filePath: req.filePath,
        question: req.question,
        roster: req.roster ?? [],
        rosterSpans: req.rosterSpans ?? [],
        capsules: req.capsules ?? [],
      }),
    )
  }
  sock.onmessage = (ev) => {
    let frame: QueryFrame
    try {
      frame = JSON.parse(ev.data as string) as QueryFrame
    } catch {
      return
    }
    if (frame.kind === 'delta') h.onDelta(frame.text)
    else if (frame.kind === 'done') {
      settled = true
      h.onDone()
      close()
    } else if (frame.kind === 'error') {
      settled = true
      h.onError(frame.message)
      close()
    }
  }
  sock.onerror = () => {
    if (settled) return
    settled = true
    h.onError('连接失败')
    close()
  }
  sock.onclose = () => {
    if (settled) return
    settled = true
    h.onError('连接已关闭')
  }
  return {
    cancel: () => {
      settled = true
      close()
    },
  }
}
