// Ghost-annotation wire types — mirror the Rust domain types (cache_store.rs /
// 技术方案 §3) and the WS frame protocol (routes.rs GenFrame, §4). Hand-written
// for now; ts-rs generation (ADR-0013) is not yet wired.

/** Function-granularity semantic capsule (技术方案 §3). */
export interface Capsule {
  fnId: string
  signature: string
  summary: string
  complexity: string
  io: string
  orientationId: string
  role: FunctionRole
}

/** Line-level ghost annotation on a key line (技术方案 §3). */
export interface LineAnnotation {
  fnId: string
  lineNumber: number
  text: string
  color: string
}

/** Source-backed file-orientation types. These mirror orientation.rs; every
 * generated Capsule carries the exact backend-selected FunctionRole. */
export interface CodeEvidenceRef {
  id: string
  filePath: string
  startLine: number
  endLine: number
  symbol?: string
}

export type ActorBoundary = 'inside-file' | 'project' | 'external'

export interface OrientationActor {
  id: string
  name: string
  role: string
  boundary: ActorBoundary
}

export interface OrientationType {
  name: string
  ownerActorId: string
  meaning: string
}

export interface OrientationFlowStep {
  fromActorId: string
  via: string
  payload: string
  toActorId: string
  why: string
  evidenceIds: string[]
}

export type OrientationFlowKind = 'request' | 'response' | 'control' | 'stats' | 'other'

export interface OrientationFlow {
  id: string
  name: string
  kind: OrientationFlowKind
  why: string
  steps: OrientationFlowStep[]
}

export type FunctionLane = 'core' | 'supporting'

export interface FunctionRole {
  fnId: string
  lane: FunctionLane
  flowIds: string[]
  stage: string
  receivesFromActorIds: string[]
  consumes: string[]
  sendsToActorIds: string[]
  produces: string[]
  why: string
  evidenceIds: string[]
}

export interface SupportingCapability {
  name: string
  why: string
  functionIds: string[]
  evidenceIds: string[]
}

export interface OrientationWalkthrough {
  title: string
  input: string
  steps: { text: string; evidenceIds: string[] }[]
}

export interface OrientationInvariant {
  text: string
  evidenceIds: string[]
}

export type OrientationCoverageMode = 'full-source' | 'bounded-source'

export interface FileOrientationCard {
  schemaVersion: number
  orientationId: string
  filePath: string
  purpose: string
  actors: OrientationActor[]
  types: OrientationType[]
  coreFlows: OrientationFlow[]
  supportingCapabilities: SupportingCapability[]
  functionRoles: FunctionRole[]
  walkthrough: OrientationWalkthrough
  invariants: OrientationInvariant[]
  evidence: CodeEvidenceRef[]
  coverage: {
    mode: OrientationCoverageMode
    omittedFunctionIds: string[]
  }
}

export type OrientationPhase = 'planning-source' | 'orienting'

/** One inbound frame from `WS /api/orient` (S-ORI-4). The backend echoes
 * `reqId`; only `done` after a matching `card` opens the capsule gate. */
export type OrientationFrame =
  | { kind: 'cache-hit'; reqId: string }
  | { kind: 'status'; reqId: string; phase: OrientationPhase; message: string }
  | { kind: 'card'; reqId: string; card: FileOrientationCard }
  | { kind: 'done'; reqId: string }
  | { kind: 'error'; reqId: string; message: string }

/** User-visible lifecycle of the current file's function generation. */
export type GenerationPhase = 'idle' | 'running' | 'paused' | 'done'

export interface GenerationProgress {
  phase: GenerationPhase
  completed: number
  total: number
}

/** One inbound frame from `WS /api/generate` (S7a, §4). `reqId` echoes the
 *  request (= the function id); terminal frames are `done` | `error`. */
export type GenFrame =
  | { kind: 'cache-hit'; reqId: string }
  | { kind: 'capsule'; reqId: string; capsule: Capsule }
  | { kind: 'line'; reqId: string; line: LineAnnotation }
  | { kind: 'done'; reqId: string }
  | { kind: 'error'; reqId: string; message: string }

/** Evidence metadata shared by selection explanations and follow-up queries.
 * `web-uncited` is a successful supplier search with no returned URLs. */
export type EvidenceStatus = 'project-source' | 'web-cited' | 'web-uncited' | 'unverified'

export interface SourceLink {
  title: string
  url: string
}

export type QueryPhase =
  | 'planning-source'
  | 'planning-web'
  | 'searching-web'
  | 'answering'
  | 'fallback'

/** Backend-built structural preview for one query. All E# references are local
 * to this request and must resolve through `evidence`; graph-only relations are
 * never promoted into `direction`. */
export interface QueryMap {
  actors: OrientationActor[]
  direction: OrientationFlowStep[]
  coreFunctionIds: string[]
  supportingFunctionIds: string[]
  walkthrough: OrientationWalkthrough
  evidence: CodeEvidenceRef[]
}

/** One completed question/answer pair in the in-memory follow-up trace. */
export interface QueryTurn {
  question: string
  answer: string
  codeEvidenceIds: string[]
}

/** Replayable, scope-bound follow-up context. The original question is stored
 * separately so backend prompt trimming can always retain it while dropping old
 * complete turns from the middle. */
export interface QueryTrace {
  scopeKey: string
  scopeRevision: string
  originalQuestion: string
  turns: QueryTurn[]
}

/** One inbound frame from either query WebSocket (S-QMAP-1, routes.rs
 * QueryFrame). `status* -> map -> evidence?` precedes the free-form markdown
 * `delta` stream; terminal frames remain `done` | `error`. */
export type QueryFrame =
  | { kind: 'status'; reqId: string; phase: QueryPhase; message: string }
  | { kind: 'map'; reqId: string; map: QueryMap }
  | {
      kind: 'evidence'
      reqId: string
      status: EvidenceStatus
      sources?: SourceLink[]
      warning?: string
    }
  | { kind: 'delta'; reqId: string; text: string }
  | { kind: 'done'; reqId: string }
  | { kind: 'error'; reqId: string; message: string }

export type SelectionKind = '模块' | '类型' | '函数' | '方法' | '变量' | '表达式' | '未知'

export interface SelectionExplanation {
  selectedText: string
  kind: SelectionKind
  meaning: string
  roleHere: string
  origin?: string
  evidenceStatus: EvidenceStatus
  sources?: SourceLink[]
  warning?: string
}

export type SelectionPhase =
  | 'resolving-project'
  | 'planning-web'
  | 'searching-web'
  | 'answering'
  | 'fallback'

/** One inbound frame from `WS /api/explain-selection` (S-SEL-1). */
export type SelectionFrame =
  | { kind: 'cache-hit'; reqId: string }
  | { kind: 'status'; reqId: string; phase: SelectionPhase; message: string }
  | { kind: 'result'; reqId: string; explanation: SelectionExplanation }
  | { kind: 'done'; reqId: string }
  | { kind: 'error'; reqId: string; message: string }
