<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import { actionLabels, type ActionFailure } from '../shared/workspace'
import { errorGuidance } from '../shared/error-guidance'
import type { Action } from '../shared/types'
const props = defineProps<{
  failure: ActionFailure
  busy: boolean
  running: boolean
  preview: boolean
  displayOnly?: boolean
  emergencyBusy?: boolean
  restoreToken?: string
}>()
defineEmits<{
  dismiss: []
  export: []
  stop: []
  navigate: [page: string]
  action: [action: Action]
}>()
const stage = computed(() =>
  props.displayOnly
    ? props.failure.action === 'start'
      ? '启用显示器切换'
      : props.failure.action === 'switch'
        ? '切换显示器输入'
        : actionLabels[props.failure.action] || '运行显示器切换'
    : actionLabels[props.failure.action] || '运行共享功能',
)
const guidance = computed(() => errorGuidance(props.failure))
const destination = computed(() =>
  props.displayOnly && guidance.value.destination === 'pairing'
    ? 'display'
    : guidance.value.destination,
)
</script>
<template>
  <Teleport to="body">
    <div class="runtime-error-layer">
      <section class="runtime-error" aria-labelledby="error-title" role="alert">
        <AppIcon name="alert" :size="21" />
        <div class="error-body">
          <h2 id="error-title">{{ stage }}遇到问题</h2>
          <p class="error-reason copyable-text">{{ guidance.summary }}</p>
          <p>{{ guidance.advice }}</p>
          <details>
            <summary>查看错误详情</summary>
            <pre class="copyable-text">{{ failure.message }}</pre>
          </details>
          <div class="button-group">
            <button
              v-if="failure.retry"
              class="button primary compact"
              :disabled="busy || preview"
              @click="$emit('action', failure.retry)"
            >
              重试此操作
            </button>
            <button
              v-if="restoreToken && failure.action !== 'restoreOutput'"
              class="button secondary compact"
              :disabled="busy || preview"
              @click="$emit('action', { type: 'restoreOutput', token: restoreToken })"
            >
              恢复输出
            </button>
            <button
              class="button secondary compact"
              :disabled="busy || preview"
              @click="$emit('export')"
            >
              导出诊断日志</button
            ><button class="text-button" @click="$emit('navigate', destination)">
              {{
                displayOnly && guidance.destination === 'pairing'
                  ? '检查显示器配置'
                  : guidance.label
              }}</button
            ><button
              v-if="running"
              class="text-button"
              :disabled="emergencyBusy"
              @click="$emit('stop')"
            >
              {{
                emergencyBusy ? '正在恢复…' : displayOnly ? '停用显示器切换' : '停止共享并恢复输出'
              }}</button
            ><button class="text-button" @click="$emit('navigate', 'activity')">
              查看活动记录
            </button>
          </div>
        </div>
        <button
          class="icon-button dismiss-error"
          aria-label="关闭错误提示"
          @click="$emit('dismiss')"
        >
          <AppIcon name="close" :size="18" />
        </button>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.runtime-error-layer {
  position: fixed;
  z-index: 90;
  top: 82px;
  right: 24px;
  width: min(520px, calc(100vw - 40px));
  max-height: calc(100dvh - 106px);
  overflow-y: auto;
  border-radius: 14px;
  box-shadow:
    0 12px 40px rgb(64 44 24 / 14%),
    0 2px 8px rgb(64 44 24 / 6%);
  animation: error-appear 0.18s ease-out;
}
@keyframes error-appear {
  from {
    opacity: 0;
    transform: translateY(-6px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}
@media (prefers-reduced-motion: reduce) {
  .runtime-error-layer {
    animation: none;
  }
}
</style>
