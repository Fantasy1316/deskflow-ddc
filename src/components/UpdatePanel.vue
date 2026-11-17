<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import { useUpdateCheck } from '../composables/useUpdateCheck'
import { projectLinks } from '../transport/project-links'

const props = defineProps<{ currentVersion: string }>()
const emit = defineEmits<{ openReleases: [event: MouseEvent] }>()
const { state, check } = useUpdateCheck(props.currentVersion)
const result = computed(() => (state.value.status === 'checked' ? state.value.result : null))
const title = computed(() => {
  if (state.value.status === 'checking') return '正在检查最新正式版…'
  if (state.value.status === 'error') return '暂时无法检查更新'
  if (result.value?.status === 'available') return `发现新版本 v${result.value.version}`
  if (result.value?.status === 'current') return '当前已是最新正式版'
  if (result.value?.status === 'ahead') return '当前版本高于最新正式版'
  return '检查版本更新'
})
const description = computed(() => {
  if (state.value.status === 'error') return state.value.message
  if (result.value?.status === 'available') return '升级后可体验最新改进，两端请使用相同版本。'
  if (result.value) return `GitHub 最新正式版：v${result.value.version}`
  return '点击检查，从 GitHub 获取最新正式版信息。'
})
const publishedDate = computed(() =>
  result.value?.publishedAt ? new Date(result.value.publishedAt).toLocaleDateString('zh-CN') : '',
)
</script>

<template>
  <section
    class="panel update-panel"
    aria-labelledby="update-title"
    :aria-busy="state.status === 'checking'"
  >
    <div class="update-row">
      <span class="section-icon"
        ><AppIcon name="refresh" :size="20" :class="{ spinning: state.status === 'checking' }"
      /></span>
      <div class="update-text" aria-live="polite" aria-atomic="true">
        <h2 id="update-title">{{ title }}</h2>
        <p>{{ description }}</p>
      </div>
      <button
        class="button secondary update-check"
        :disabled="state.status === 'checking'"
        @click="check"
      >
        {{
          state.status === 'checking'
            ? '正在检查…'
            : state.status === 'error'
              ? '重新检查'
              : '检查更新'
        }}
      </button>
    </div>
    <div v-if="result || state.status === 'error'" class="update-result">
      <div class="update-meta">
        <span v-if="publishedDate">发布于 {{ publishedDate }}</span>
        <span v-else>当前版本 v{{ currentVersion }}</span>
        <a
          class="text-button update-download"
          :href="projectLinks.releases"
          target="_blank"
          rel="noopener noreferrer"
          @click="emit('openReleases', $event)"
        >
          {{ result?.status === 'available' ? '前往下载' : '查看版本页面' }}
          <AppIcon name="external" :size="13" />
        </a>
      </div>
      <details v-if="result?.status === 'available' && result.notes" class="update-notes">
        <summary>更新说明</summary>
        <p>{{ result.notes }}</p>
      </details>
    </div>
  </section>
</template>

<style scoped>
.update-panel {
  margin-top: 20px;
  padding: 23px 25px;
}
.update-row {
  display: flex;
  align-items: center;
  gap: 13px;
}
.update-row .section-icon {
  flex-shrink: 0;
}
.update-text {
  flex: 1;
  min-width: 0;
}
.update-text h2 {
  font-size: 15px;
  overflow-wrap: anywhere;
}
.update-text p {
  margin-top: 5px;
  color: var(--muted);
  font-size: 12px;
  line-height: 1.7;
}
.update-check {
  flex-shrink: 0;
}
.update-result {
  margin-top: 16px;
  padding-top: 14px;
  border-top: 1px solid var(--line);
}
.update-meta {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  color: var(--muted);
  font-size: 12px;
}
.update-download {
  cursor: pointer;
  text-decoration: none;
}
.update-notes {
  margin-top: 14px;
  font-size: 12px;
  color: #53655b;
}
.update-notes summary {
  width: fit-content;
  cursor: pointer;
  color: var(--green);
}
.update-notes p {
  margin-top: 12px;
  max-height: 250px;
  overflow-y: auto;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  line-height: 1.8;
}
.update-panel :is(a, button, summary):focus-visible {
  outline: 3px solid #bdd2c2;
  outline-offset: 3px;
}
.spinning {
  animation: update-spin 1s linear infinite;
}
@keyframes update-spin {
  to {
    transform: rotate(360deg);
  }
}
@media (max-width: 600px) {
  .update-panel {
    padding: 22px;
  }
  .update-row {
    flex-wrap: wrap;
  }
  .update-check {
    margin-left: 51px;
  }
}
@media (prefers-reduced-motion: reduce) {
  .spinning {
    animation: none;
  }
}
</style>
