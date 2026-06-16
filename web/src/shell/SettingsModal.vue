<script setup lang="ts">
// SettingsModal — the LLM backend settings dialog (U5b, ADR-0018). On open it
// GETs the current (masked) config; saving POSTs the new values, which the
// backend applies live (no restart) and writes back to .env. The API key is
// write-only: the field starts blank and only overwrites when typed (an empty
// field keeps the existing key, per the masked hint). Standard IDE chrome.
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { getLlmSettings, saveLlmSettings } from '../api'

const emit = defineEmits<{ close: [] }>()

const baseUrl = ref('')
const model = ref('')
const keyStatus = ref<'set' | 'unset'>('unset')
const keyHint = ref<string | null>(null)
const apiKey = ref('') // write-only; blank = keep the existing key

const loading = ref(true)
const saving = ref(false)
const error = ref('')
const saved = ref(false)

onMounted(async () => {
  window.addEventListener('keydown', onKey)
  try {
    const s = await getLlmSettings()
    baseUrl.value = s.baseUrl
    model.value = s.model
    keyStatus.value = s.keyStatus
    keyHint.value = s.keyHint
  } catch (e) {
    error.value = String(e)
  } finally {
    loading.value = false
  }
})
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}

const canSave = () => !saving.value && !!baseUrl.value.trim() && !!model.value.trim()

async function save() {
  if (!canSave()) return
  saving.value = true
  error.value = ''
  saved.value = false
  try {
    const s = await saveLlmSettings({
      baseUrl: baseUrl.value.trim(),
      model: model.value.trim(),
      apiKey: apiKey.value.trim() || undefined,
    })
    keyStatus.value = s.keyStatus
    keyHint.value = s.keyHint
    apiKey.value = '' // never retain the typed secret
    saved.value = true
  } catch (e) {
    error.value = String(e)
  } finally {
    saving.value = false
  }
}

const keyPlaceholder = () =>
  keyStatus.value === 'set'
    ? `已配置 ${keyHint.value ?? ''} — 留空＝保持不变`
    : '未配置 — 输入以启用'
</script>

<template>
  <div class="modal-overlay" @click.self="emit('close')">
    <section class="modal settings-modal" role="dialog" aria-label="设置 · LLM 后端">
      <header class="modal-head">
        <span class="modal-title">设置 · LLM 后端</span>
        <button class="modal-close" type="button" aria-label="关闭" @click="emit('close')">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <path d="M6 6l12 12M18 6L6 18" />
          </svg>
        </button>
      </header>

      <div v-if="loading" class="modal-body settings-loading">加载中…</div>
      <form v-else class="modal-body settings-form" @submit.prevent="save">
        <label class="settings-field">
          <span class="settings-label">Base URL</span>
          <input v-model="baseUrl" class="settings-input" placeholder="https://api.example.com/v1" />
        </label>
        <label class="settings-field">
          <span class="settings-label">Model</span>
          <input v-model="model" class="settings-input" placeholder="glm-5.1" />
        </label>
        <label class="settings-field">
          <span class="settings-label">API Key</span>
          <input
            v-model="apiKey"
            class="settings-input"
            type="password"
            autocomplete="off"
            :placeholder="keyPlaceholder()"
          />
        </label>
        <p class="settings-note">
          仅支持 OpenAI 兼容端点。密钥写回本地 .env,不上传、不回显;留空即保持当前密钥。
        </p>
        <p v-if="error" class="settings-error">{{ error }}</p>
        <p v-else-if="saved" class="settings-saved">✓ 已保存 — 已即时生效(无需重启)</p>
        <div class="settings-actions">
          <button type="button" class="settings-btn" @click="emit('close')">关闭</button>
          <button type="submit" class="settings-btn primary" :disabled="!canSave()">
            {{ saving ? '保存中…' : '保存' }}
          </button>
        </div>
      </form>
    </section>
  </div>
</template>
