import type { Capsule, FunctionRole } from './ghostTypes'

export interface CapsulePresentation {
  lane: '核心' | '外围'
  stage: string
  direction: string
  why: string
  evidence: string
}

function unique(values: string[]): string[] {
  return [...new Set(values.filter((value) => value.trim().length > 0))]
}

/** Render only canonical actor IDs from the backend-bound FunctionRole. */
export function roleDirection(role: FunctionRole): string {
  const sources = unique(role.receivesFromActorIds)
  const targets = unique(role.sendsToActorIds)
  if (sources.length > 0 && targets.length > 0) {
    return `${sources.join(' / ')} → ${targets.join(' / ')}`
  }
  if (sources.length > 0) return `${sources.join(' / ')} → 本函数`
  if (targets.length > 0) return `本函数 → ${targets.join(' / ')}`
  return '无跨参与者流'
}

export function capsulePresentation(capsule: Capsule): CapsulePresentation {
  return {
    lane: capsule.role.lane === 'core' ? '核心' : '外围',
    stage: capsule.role.stage || '未标注阶段',
    direction: roleDirection(capsule.role),
    why: capsule.role.why || '未提供角色依据',
    evidence:
      unique(capsule.role.evidenceIds).join(' · ') || '无源码证据锚点',
  }
}
