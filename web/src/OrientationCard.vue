<script setup lang="ts">
import type { ActorBoundary, FileOrientationCard } from './ghostTypes'
import type { OrientationViewState } from './orientationState'

const props = defineProps<{ state: OrientationViewState }>()
defineEmits<{ retry: [] }>()

function actorName(id: string): string {
  return props.state.card?.actors.find((actor) => actor.id === id)?.name ?? id
}

function boundaryLabel(boundary: ActorBoundary): string {
  switch (boundary) {
    case 'inside-file':
      return '文件内'
    case 'project':
      return '项目内'
    case 'external':
      return '外部'
  }
}

function coverageLabel(card: FileOrientationCard): string {
  return card.coverage.mode === 'full-source' ? '整源核验' : '有界取源'
}
</script>

<template>
  <section
    v-if="state.mode !== 'idle'"
    class="orientation-shell"
    :data-orientation-mode="state.mode"
    :data-orientation-file="state.filePath"
  >
    <div
      v-if="state.mode === 'loading' && !state.card"
      class="orientation-pending"
      data-testid="orientation-pending"
      role="status"
    >
      <span class="orientation-spinner" aria-hidden="true"></span>
      <span class="orientation-pending-copy">
        <strong>正在建立文件定向</strong>
        <span>{{ state.message }}</span>
      </span>
      <span v-if="state.cacheHit" class="orientation-badge accent">缓存命中</span>
    </div>

    <div
      v-else-if="state.mode === 'error'"
      class="orientation-error"
      data-testid="orientation-error"
      role="alert"
    >
      <span class="orientation-error-copy">
        <strong>文件定向失败</strong>
        <span>{{ state.errorMessage }}</span>
      </span>
      <button type="button" data-testid="orientation-retry" @click="$emit('retry')">
        重试
      </button>
    </div>

    <details
      v-else-if="state.card"
      class="orientation-card"
      data-testid="orientation-card"
      open
    >
      <summary>
        <span class="orientation-title">文件定向</span>
        <span class="orientation-badge">{{ coverageLabel(state.card) }}</span>
        <span v-if="state.cacheHit" class="orientation-badge accent">缓存命中</span>
        <span v-if="state.mode === 'loading'" class="orientation-confirming">
          正在完成激活检查
        </span>
      </summary>

      <div class="orientation-body">
        <p class="orientation-purpose">{{ state.card.purpose }}</p>

        <div class="orientation-overview">
          <section>
            <h3>参与者</h3>
            <ul class="orientation-actors">
              <li v-for="actor in state.card.actors" :key="actor.id">
                <span class="orientation-actor-name">{{ actor.name }}</span>
                <span class="orientation-boundary">{{ boundaryLabel(actor.boundary) }}</span>
                <span>{{ actor.role }}</span>
              </li>
            </ul>
          </section>

          <section>
            <h3>{{ state.card.walkthrough.title }}</h3>
            <p class="orientation-input">
              输入：<code>{{ state.card.walkthrough.input }}</code>
            </p>
            <ol class="orientation-walkthrough">
              <li v-for="(step, index) in state.card.walkthrough.steps" :key="index">
                {{ step.text }}
              </li>
            </ol>
          </section>
        </div>

        <section class="orientation-flows">
          <h3>核心方向</h3>
          <article v-for="flow in state.card.coreFlows" :key="flow.id">
            <div class="orientation-flow-head">
              <strong>{{ flow.name }}</strong>
              <span>{{ flow.why }}</span>
            </div>
            <div
              v-for="(step, index) in flow.steps"
              :key="`${flow.id}-${index}`"
              class="orientation-flow-step"
            >
              <span>{{ actorName(step.fromActorId) }}</span>
              <span class="orientation-arrow">— {{ step.payload }} / {{ step.via }} →</span>
              <span>{{ actorName(step.toActorId) }}</span>
              <span class="orientation-why">{{ step.why }}</span>
              <span class="orientation-evidence">{{ step.evidenceIds.join(' · ') }}</span>
            </div>
          </article>
        </section>

        <section v-if="state.card.supportingCapabilities.length" class="orientation-supporting">
          <h3>外围能力</h3>
          <ul>
            <li v-for="capability in state.card.supportingCapabilities" :key="capability.name">
              <strong>{{ capability.name }}</strong>：{{ capability.why }}
            </li>
          </ul>
        </section>

        <p class="orientation-footnote">
          {{ state.card.evidence.length }} 个源码证据锚点
          <template v-if="state.card.coverage.omittedFunctionIds.length">
            · {{ state.card.coverage.omittedFunctionIds.length }} 个函数体未纳入本次有界核验
          </template>
        </p>
      </div>
    </details>
  </section>
</template>
