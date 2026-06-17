// Thin fetch wrappers over the Rust backend's L0 endpoints (技术方案 §4).
// Requests go to /api/* and are proxied to 127.0.0.1:7878 in dev (vite.config.ts).

import type { FunctionSpan } from './parser/types.ts'
import type { LineAnnotation, QueryFrame } from './ghostTypes'
import type { CapsuleSummary } from './queryContext'

export type Lang = 'py' | 'rs' | 'md' | 'other'

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

/** The LLM backend settings the frontend can see (U5b, ADR-0018). `keyStatus` +
 *  `keyHint` are all that is ever exposed of the key (write-only): the full key
 *  never leaves the backend. `keyHint` is a masked tail like `···1234` or null. */
export interface LlmSettings {
  baseUrl: string
  model: string
  keyStatus: 'set' | 'unset'
  keyHint: string | null
}

/** GET /api/settings/llm -> the current (masked) LLM backend config. */
export async function getLlmSettings(): Promise<LlmSettings> {
  const res = await fetch('/api/settings/llm')
  if (!res.ok) throw new Error((await res.text()) || `/api/settings/llm -> ${res.status}`)
  return (await res.json()) as LlmSettings
}

/** POST /api/settings/llm -> apply new config (hot-rebuilds the backend proxy +
 *  writes .env). Omit `apiKey` (or leave it blank) to keep the existing key —
 *  the UI never has to echo the secret to change the other fields. Returns the
 *  updated masked settings. */
export async function saveLlmSettings(req: {
  baseUrl: string
  model: string
  apiKey?: string
}): Promise<LlmSettings> {
  const res = await fetch('/api/settings/llm', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(req),
  })
  if (!res.ok) throw new Error((await res.text()) || `/api/settings/llm -> ${res.status}`)
  return (await res.json()) as LlmSettings
}

/** POST /api/settings/llm/test -> probe the given backend with one minimal
 *  completion before saving (U5c). Omit `apiKey` (or leave it blank) to test with
 *  the currently-stored key. Returns `{ ok }` on success or `{ ok: false, error }`
 *  with the backend's failure message; the HTTP call itself is always 200. */
export async function testLlmSettings(req: {
  baseUrl: string
  model: string
  apiKey?: string
}): Promise<{ ok: boolean; error?: string }> {
  const res = await fetch('/api/settings/llm/test', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(req),
  })
  if (!res.ok) throw new Error((await res.text()) || `/api/settings/llm/test -> ${res.status}`)
  return (await res.json()) as { ok: boolean; error?: string }
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

/** Callbacks for a streaming document translation (文档翻译). A cache hit fires
 *  `onCached` (whole doc) then `onDone`; a miss fires `onTotal` then `onChunk` per
 *  chunk in order (code already restored; `ok=false` means that block kept its
 *  English original) then `onDone`. `onError` is terminal (no project / unconfigured
 *  LLM / all chunks failed). */
export interface TranslateHandlers {
  onCached: (text: string) => void
  onTotal: (total: number) => void
  onChunk: (index: number, text: string, ok: boolean) => void
  onDone: () => void
  onError: (message: string) => void
}

/** Handle to an in-flight translation; `cancel` tears the socket down silently. */
export interface TranslateStream {
  cancel: () => void
}

type TranslateFrame =
  | { kind: 'cached'; text: string }
  | { kind: 'total'; total: number }
  | { kind: 'chunk'; index: number; text: string; ok: boolean }
  | { kind: 'done' }
  | { kind: 'error'; message: string }

/** Open `WS /api/translate`, request a file's translation, and stream the result
 *  back chunk by chunk for live progress + incremental rendering (文档翻译). One
 *  socket per request; closed on the terminal frame or on `cancel` (file switch /
 *  unmount). Reopening an unchanged file hits the .fluid/ cache (single `cached`). */
export function streamTranslate(filePath: string, h: TranslateHandlers): TranslateStream {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const sock = new WebSocket(`${proto}://${location.host}/api/translate`)
  let settled = false
  const close = () => {
    try {
      sock.close()
    } catch {
      /* already closing */
    }
  }
  sock.onopen = () => sock.send(JSON.stringify({ filePath }))
  sock.onmessage = (ev) => {
    let f: TranslateFrame
    try {
      f = JSON.parse(ev.data as string) as TranslateFrame
    } catch {
      return
    }
    switch (f.kind) {
      case 'cached':
        h.onCached(f.text)
        break
      case 'total':
        h.onTotal(f.total)
        break
      case 'chunk':
        h.onChunk(f.index, f.text, f.ok)
        break
      case 'done':
        settled = true
        h.onDone()
        close()
        break
      case 'error':
        settled = true
        h.onError(f.message)
        close()
        break
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
