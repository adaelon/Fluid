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
}

/** Line-level ghost annotation on a key line (技术方案 §3). */
export interface LineAnnotation {
  fnId: string
  lineNumber: number
  text: string
  color: string
}

/** Source-backed file-orientation types. These mirror orientation.rs and are
 * deliberately separate from Capsule: S-ORI-4 only gates activation and does
 * not yet bind child products to orientationId (that is S-CAP-1). */
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

export type QueryPhase = 'planning-web' | 'searching-web' | 'answering' | 'fallback'

/** One inbound frame from either query WebSocket (S-QWEB-2, routes.rs
 * QueryFrame). `status` and `evidence` precede the existing free-form markdown
 * `delta` stream; terminal frames remain `done` | `error`. */
export type QueryFrame =
  | { kind: 'status'; reqId: string; phase: QueryPhase; message: string }
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
