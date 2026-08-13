// Thin fetch wrappers over the Rust backend's L0 endpoints (技术方案 §4).
// Requests go to /api/* and are proxied to 127.0.0.1:7878 in dev (vite.config.ts).

import type { FunctionSpan } from './parser/types.ts'
import type {
  EvidenceStatus,
  LineAnnotation,
  OrientationFrame,
  QueryFrame,
  QueryMap,
  SelectionFrame,
  SourceLink,
} from './ghostTypes'
import type { CapsuleSummary } from './queryContext'

export type Lang = 'py' | 'rs' | 'md' | 'other'

export interface FileNode {
  path: string
  name: string
  lang: Lang
}

export type ReadingAnchor =
  | {
      kind: 'code'
      topLine: number
      offsetPx: number
      totalLines: number
    }
  | {
      kind: 'markdown'
      blockDigest: string
      occurrence: number
      offsetPx: number
      scrollRatio: number
    }

export interface ProjectReadingSnapshot {
  expandedDirectories: string[]
  openFiles: string[]
  activeFile: string | null
  readingPositions: Record<string, ReadingAnchor>
}

export type ReadingStateWarningKind =
  | 'corrupt-json'
  | 'unsupported-schema'
  | 'project-root-mismatch'
  | 'invalid-path'
  | 'invalid-value'
  | 'record-too-large'
  | 'invalid-record'
  | 'io'

export interface ReadingStateWarning {
  kind: ReadingStateWarningKind
  file: string
  message: string
}

export interface CurrentWorkspaceResponse {
  projectRoot: string | null
  snapshot: ProjectReadingSnapshot | null
  warnings: ReadingStateWarning[]
}

export interface OpenProjectResponse {
  root: string
  snapshot: ProjectReadingSnapshot | null
  warnings: ReadingStateWarning[]
}

export class WorkspaceApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message)
    this.name = 'WorkspaceApiError'
  }
}

async function workspaceResponseError(res: Response, endpoint: string): Promise<never> {
  const detail = (await res.text()).trim()
  throw new WorkspaceApiError(res.status, detail || `${endpoint} -> ${res.status}`)
}

export type QueryScopeSpec =
  | { kind: 'current'; paths: [string] }
  | { kind: 'selected'; paths: [string, string, ...string[]] }

export type QueryThreadFreshness = 'fresh' | 'stale'
export type QueryThreadStaleReason = 'source-changed' | 'source-missing'

export interface PersistedQueryEvidence {
  status: EvidenceStatus
  sources: SourceLink[]
  warning?: string
}

export interface PersistedQueryTurn {
  question: string
  answer: string
  map: QueryMap
  evidence: PersistedQueryEvidence | null
  codeEvidenceIds: string[]
  completedAt: string
}

export interface QueryThread {
  schemaVersion: 1
  id: string
  title: string
  createdAt: string
  updatedAt: string
  scope: QueryScopeSpec
  sourceRevision: string
  originalQuestion: string
  turns: PersistedQueryTurn[]
  freshness: QueryThreadFreshness
  staleReason?: QueryThreadStaleReason
}

export interface QueryThreadSummary {
  id: string
  title: string
  updatedAt: string
  scope: QueryScopeSpec
  turnCount: number
  freshness: QueryThreadFreshness
  staleReason?: QueryThreadStaleReason
}

export interface QueryThreadWarning {
  file: string
  message: string
}

export interface QueryThreadListResponse {
  threads: QueryThreadSummary[]
  warnings: QueryThreadWarning[]
}

type PersistedQueryEvidenceWire = Omit<PersistedQueryEvidence, 'sources'> & {
  sources?: unknown
}

type PersistedQueryTurnWire = Omit<PersistedQueryTurn, 'evidence'> & {
  evidence: PersistedQueryEvidenceWire | null
}

type QueryThreadWire = Omit<QueryThread, 'turns'> & {
  turns: PersistedQueryTurnWire[]
}

export class QueryHistoryApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message)
    this.name = 'QueryHistoryApiError'
  }
}

export class QueryHistoryContractError extends Error {
  constructor(
    readonly endpoint: string,
    detail: string,
  ) {
    super(`${endpoint} returned an invalid query thread: ${detail}`)
    this.name = 'QueryHistoryContractError'
  }
}

async function queryHistoryResponseError(res: Response, endpoint: string): Promise<never> {
  const detail = (await res.text()).trim()
  throw new QueryHistoryApiError(res.status, detail || `${endpoint} -> ${res.status}`)
}

function normalizePersistedQueryEvidence(
  wire: PersistedQueryEvidenceWire,
  endpoint: string,
  turnIndex: number,
): PersistedQueryEvidence {
  const { sources, ...metadata } = wire
  if (sources !== undefined && !Array.isArray(sources)) {
    throw new QueryHistoryContractError(
      endpoint,
      `turns[${turnIndex}].evidence.sources must be an array when present`,
    )
  }
  return {
    ...metadata,
    sources: sources ?? [],
  }
}

function normalizeQueryThread(wire: QueryThreadWire, endpoint: string): QueryThread {
  return {
    ...wire,
    turns: wire.turns.map((turn, turnIndex) => ({
      ...turn,
      evidence: turn.evidence
        ? normalizePersistedQueryEvidence(turn.evidence, endpoint, turnIndex)
        : null,
    })),
  }
}

async function queryThreadDetailResponse(res: Response, endpoint: string): Promise<QueryThread> {
  if (!res.ok) return queryHistoryResponseError(res, endpoint)
  return normalizeQueryThread((await res.json()) as QueryThreadWire, endpoint)
}

/** Create the zero-turn durable record required before either query socket can stream. */
export async function createQueryThread(req: {
  scope: QueryScopeSpec
  originalQuestion: string
}): Promise<QueryThread> {
  const endpoint = '/api/query-threads'
  const res = await fetch(endpoint, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(req),
  })
  return queryThreadDetailResponse(res, endpoint)
}

/** Load newest-first project summaries plus isolated bad-record warnings. */
export async function listQueryThreads(): Promise<QueryThreadListResponse> {
  const endpoint = '/api/query-threads'
  const res = await fetch(endpoint)
  if (!res.ok) return queryHistoryResponseError(res, endpoint)
  return (await res.json()) as QueryThreadListResponse
}

/** Load one complete persisted thread for reading or a fresh continuation. */
export async function getQueryThread(threadId: string): Promise<QueryThread> {
  const endpoint = `/api/query-threads/${encodeURIComponent(threadId)}`
  const res = await fetch(endpoint)
  return queryThreadDetailResponse(res, endpoint)
}

/** Delete exactly one validated thread from the current project. */
export async function deleteQueryThread(threadId: string): Promise<void> {
  const endpoint = `/api/query-threads/${encodeURIComponent(threadId)}`
  const res = await fetch(endpoint, { method: 'DELETE' })
  if (!res.ok) return queryHistoryResponseError(res, endpoint)
}

/** Rebind a source-changed thread's original question and scope to current
 * project bytes. The backend returns a distinct fresh record with zero turns. */
export async function forkQueryThreadCurrent(threadId: string): Promise<QueryThread> {
  const endpoint = `/api/query-threads/${encodeURIComponent(threadId)}/fork-current`
  const res = await fetch(endpoint, { method: 'POST' })
  return queryThreadDetailResponse(res, endpoint)
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

/** GET the canonical current root and its independently persisted reading state. */
export async function fetchCurrentWorkspace(): Promise<CurrentWorkspaceResponse> {
  const endpoint = '/api/workspace/current'
  const res = await fetch(endpoint)
  if (!res.ok) return workspaceResponseError(res, endpoint)
  return (await res.json()) as CurrentWorkspaceResponse
}

/** Save one complete snapshot only when its root is still the backend's current
 * root. A delayed old-root request is preserved as WorkspaceApiError(409). */
export async function saveCurrentWorkspace(req: {
  projectRoot: string
  snapshot: ProjectReadingSnapshot
}): Promise<{ saved: true }> {
  const endpoint = '/api/workspace/current'
  const body = JSON.stringify(req)
  // A small complete snapshot may outlive `pagehide`; oversized records stay a
  // normal fetch because browsers reject keepalive bodies around the 64 KiB
  // quota. Event-driven autosave remains the primary durability path either way.
  const keepalive = new TextEncoder().encode(body).byteLength <= 60 * 1024
  const res = await fetch(endpoint, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body,
    keepalive,
  })
  if (!res.ok) return workspaceResponseError(res, endpoint)
  return (await res.json()) as { saved: true }
}

/** Switch to one canonical project root and receive that target's persisted
 * snapshot in the same response, avoiding a second current-root guess. */
export async function openFolder(path: string): Promise<OpenProjectResponse> {
  const endpoint = '/api/project/open'
  const res = await fetch(endpoint, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ path }),
  })
  if (!res.ok) return workspaceResponseError(res, endpoint)
  return (await res.json()) as OpenProjectResponse
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
 *  line (S9 手动补行) or a top-level declaration (S-TS-3, when declKind is set). */
export async function explainLine(req: {
  filePath: string
  fn: FunctionSpan
  lineNumber: number
  /** Present ⇒ explain a module-level declaration (S-TS-3): `fn` carries the decl's
   *  name+span; the backend uses the decl-flavored prompt. */
  declKind?: string
}): Promise<LineAnnotation> {
  const res = await fetch('/api/explain-line', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      filePath: req.filePath,
      fn: req.fn,
      lineNumber: req.lineNumber,
      declKind: req.declKind,
    }),
  })
  if (!res.ok) throw new Error((await res.text()) || `/api/explain-line -> ${res.status}`)
  return (await res.json()) as LineAnnotation
}

export interface SelectionExplainRequest {
  reqId: string
  filePath: string
  startByte: number
  endByte: number
  rosterSpans: FunctionSpan[]
  allowWeb: boolean
  forceRefresh?: boolean
}

export interface SelectionStream {
  cancel: () => void
}

export interface OrientationRequest {
  reqId: string
  filePath: string
  rosterSpans: FunctionSpan[]
}

export interface OrientationStream {
  cancel: () => void
}

/** Open one short-lived file-orientation WebSocket. Connection failures and a
 * premature close are normalized to the same terminal `error` frame as the
 * backend protocol. Echoed reqIds are checked here and again by the reducer. */
export function streamOrientation(
  req: OrientationRequest,
  onFrame: (frame: OrientationFrame) => void,
): OrientationStream {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const sock = new WebSocket(`${proto}://${location.host}/api/orient`)
  let settled = false
  const close = () => {
    try {
      sock.close()
    } catch {
      /* already closing */
    }
  }
  const fail = (message: string) => {
    if (settled) return
    settled = true
    onFrame({ kind: 'error', reqId: req.reqId, message })
    close()
  }

  sock.onopen = () => sock.send(JSON.stringify(req))
  sock.onmessage = (event) => {
    let frame: OrientationFrame
    try {
      frame = JSON.parse(event.data as string) as OrientationFrame
    } catch {
      return
    }
    if (frame.reqId !== req.reqId) return
    onFrame(frame)
    if (frame.kind === 'done' || frame.kind === 'error') {
      settled = true
      close()
    }
  }
  sock.onerror = () => fail('连接失败')
  sock.onclose = () => fail('连接已关闭')

  return {
    cancel: () => {
      settled = true
      close()
    },
  }
}

/** Open one short-lived selection WebSocket. The backend owns source truth and
 * echoes `reqId`; mismatched frames are ignored so a stale socket cannot cross
 * into a newer selection request. */
export function streamSelectionExplanation(
  req: SelectionExplainRequest,
  onFrame: (frame: SelectionFrame) => void,
): SelectionStream {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const sock = new WebSocket(`${proto}://${location.host}/api/explain-selection`)
  let settled = false
  const close = () => {
    try {
      sock.close()
    } catch {
      /* already closing */
    }
  }
  const fail = (message: string) => {
    if (settled) return
    settled = true
    onFrame({ kind: 'error', reqId: req.reqId, message })
    close()
  }

  sock.onopen = () => sock.send(JSON.stringify(req))
  sock.onmessage = (event) => {
    let frame: SelectionFrame
    try {
      frame = JSON.parse(event.data as string) as SelectionFrame
    } catch {
      return
    }
    if (frame.reqId !== req.reqId) return
    onFrame(frame)
    if (frame.kind === 'done' || frame.kind === 'error') {
      settled = true
      close()
    }
  }
  sock.onerror = () => fail('连接失败')
  sock.onclose = () => fail('连接已关闭')

  return {
    cancel: () => {
      settled = true
      close()
    },
  }
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

/** Ordered frame callback shared by both streaming follow-up scopes. The backend
 * guarantees status* -> map -> evidence? -> delta* -> done | error. */
export interface QueryHandlers {
  onFrame: (frame: QueryFrame) => void
}

/** Handle to an in-flight query stream; `cancel` tears the socket down silently. */
export interface QueryStream {
  cancel: () => void
}

/** Open `WS /api/query`, send one question, and forward status* -> map ->
 *  evidence? -> delta×N → done | error. One socket per question; it is
 *  closed on the terminal frame or on `cancel` (file switch / unmount). The S10a
 *  backend treats roster/capsules/focus as optional; S10b-cap layers in the
 *  current file's roster + generated capsule summaries so the answer no longer
 *  leans on the graph's file_summary backstop alone. */
export function streamQuery(
  req: {
    reqId: string
    threadId: string
    filePath: string
    orientationId: string
    question: string
    roster?: string[]
    rosterSpans?: FunctionSpan[]
    capsules?: CapsuleSummary[]
    allowWeb: boolean
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
        reqId: req.reqId,
        threadId: req.threadId,
        filePath: req.filePath,
        orientationId: req.orientationId,
        question: req.question,
        roster: req.roster ?? [],
        rosterSpans: req.rosterSpans ?? [],
        capsules: req.capsules ?? [],
        allowWeb: req.allowWeb,
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
    h.onFrame(frame)
    if (frame.kind === 'done') {
      settled = true
      close()
    } else if (frame.kind === 'error') {
      settled = true
      close()
    }
  }
  sock.onerror = () => {
    if (settled) return
    settled = true
    h.onFrame({ kind: 'error', reqId: req.reqId, message: '连接失败' })
    close()
  }
  sock.onclose = () => {
    if (settled) return
    settled = true
    h.onFrame({ kind: 'error', reqId: req.reqId, message: '连接已关闭' })
  }
  return {
    cancel: () => {
      settled = true
      close()
    },
  }
}

/** Open `WS /api/query-files`, send one selected-file-set relationship question,
 *  and forward the same map-before-delta terminal contract (S-QMAP-1). */
export function streamQueryFiles(
  req: {
    reqId: string
    threadId: string
    filePaths: string[]
    question: string
    allowWeb: boolean
  },
  h: QueryHandlers,
): QueryStream {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const sock = new WebSocket(`${proto}://${location.host}/api/query-files`)
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
        reqId: req.reqId,
        threadId: req.threadId,
        filePaths: req.filePaths,
        question: req.question,
        allowWeb: req.allowWeb,
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
    h.onFrame(frame)
    if (frame.kind === 'done') {
      settled = true
      close()
    } else if (frame.kind === 'error') {
      settled = true
      close()
    }
  }
  sock.onerror = () => {
    if (settled) return
    settled = true
    h.onFrame({ kind: 'error', reqId: req.reqId, message: '连接失败' })
    close()
  }
  sock.onclose = () => {
    if (settled) return
    settled = true
    h.onFrame({ kind: 'error', reqId: req.reqId, message: '连接已关闭' })
  }
  return {
    cancel: () => {
      settled = true
      close()
    },
  }
}
