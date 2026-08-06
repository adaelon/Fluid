<script setup lang="ts">
import { computed } from 'vue'
import type { CodeEvidenceRef, QueryMap } from './ghostTypes'
import { queryEvidenceById, queryMapUnknownEvidenceIds } from './queryEvidence'

const props = defineProps<{ map: QueryMap }>()
const emit = defineEmits<{ openEvidence: [CodeEvidenceRef] }>()

const unknownEvidenceIds = computed(() => queryMapUnknownEvidenceIds(props.map))

function actorName(id: string): string {
  return props.map.actors.find((actor) => actor.id === id)?.name ?? id
}

function evidenceFor(ids: string[]): CodeEvidenceRef[] {
  return ids.flatMap((id) => {
    const reference = queryEvidenceById(props.map.evidence, id)
    return reference ? [reference] : []
  })
}

function evidenceLabel(reference: CodeEvidenceRef): string {
  const symbol = reference.symbol ? ` · ${reference.symbol}` : ''
  return `${reference.id} · ${reference.filePath}:${reference.startLine}-${reference.endLine}${symbol}`
}
</script>

<template>
  <section class="query-map" data-testid="query-map">
    <header class="query-map-head">
      <strong>方向图</strong>
      <span>{{ map.evidence.length }} 个代码证据</span>
    </header>

    <div class="query-map-actors" aria-label="参与者">
      <span v-for="actor in map.actors" :key="actor.id" class="query-map-actor">
        <strong>{{ actor.name }}</strong>
        <span>{{ actor.role }}</span>
      </span>
    </div>

    <div v-if="map.direction.length" class="query-map-direction">
      <article v-for="(step, index) in map.direction" :key="index" class="query-map-step">
        <div class="query-map-edge">
          <strong>{{ actorName(step.fromActorId) }}</strong>
          <span>— {{ step.payload }} / {{ step.via }} →</span>
          <strong>{{ actorName(step.toActorId) }}</strong>
        </div>
        <p>{{ step.why }}</p>
        <div class="query-map-evidence-links">
          <button
            v-for="reference in evidenceFor(step.evidenceIds)"
            :key="reference.id"
            type="button"
            @click="emit('openEvidence', reference)"
          >
            {{ evidenceLabel(reference) }}
          </button>
        </div>
      </article>
    </div>
    <p v-else class="query-map-empty">
      无可核验的跨组件流；以下只展示本次问题的直接源码作用。
    </p>

    <div class="query-map-lanes">
      <p>
        <strong>核心函数</strong>
        <span>{{ map.coreFunctionIds.length ? map.coreFunctionIds.join(' · ') : '未分类' }}</span>
      </p>
      <p>
        <strong>外围函数</strong>
        <span>{{ map.supportingFunctionIds.length ? map.supportingFunctionIds.join(' · ') : '无' }}</span>
      </p>
    </div>

    <div class="query-map-walkthrough">
      <strong>{{ map.walkthrough.title }}</strong>
      <span class="query-map-input">输入：{{ map.walkthrough.input }}</span>
      <ol>
        <li v-for="(step, index) in map.walkthrough.steps" :key="index">
          <span>{{ step.text }}</span>
          <span class="query-map-evidence-links">
            <button
              v-for="reference in evidenceFor(step.evidenceIds)"
              :key="reference.id"
              type="button"
              @click="emit('openEvidence', reference)"
            >
              {{ evidenceLabel(reference) }}
            </button>
          </span>
        </li>
      </ol>
    </div>

    <p v-if="unknownEvidenceIds.length" class="query-warning" role="alert">
      方向图引用了未知代码证据：{{ unknownEvidenceIds.join('、') }}。这些编号不可跳转。
    </p>
  </section>
</template>
