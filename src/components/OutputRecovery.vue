<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import { deviceName } from '../shared/workspace'
import { stageLabels, progressSteps } from '../shared/switch-progress'
import type { Action, Snapshot } from '../shared/types'
const props = defineProps<{ state: Snapshot; busy: boolean; emergencyBusy?: boolean }>()
defineEmits<{ action: [action: Action] }>()
const completed = computed(() => props.state.outputSwitch)
const recovering = computed(() => props.emergencyBusy || props.state.emergencyStopping)
const stage = computed(() =>
  recovering.value ? 'restoring-outputs' : props.state.switchProgress?.stage,
)
const steps = computed(() =>
  stage.value ? progressSteps(stage.value, props.state.config.displayMethod === 'signal') : [],
)
const disabled = computed(
  () =>
    props.busy || props.state.switching || !props.state.connected || props.state.engine !== 'ready',
)
const target = computed(() => (completed.value?.target === props.state.deviceId ? 'local' : 'peer'))
const failed = computed(() => !!completed.value && props.state.displayStatus === 'uncertain')
</script>

<template>
  <section v-if="stage" class="panel switch-progress" role="status" aria-live="polite">
    <div class="progress-heading">
      <strong>{{ recovering ? '正在停止共享并恢复输出…' : stageLabels[stage] + '…' }}</strong>
      <button
        v-if="state.running || state.switching || recovering"
        class="text-button"
        :disabled="recovering"
        @click="$emit('action', { type: 'emergencyStop' })"
      >
        {{ recovering ? '正在恢复…' : '停止共享并恢复输出' }}
      </button>
    </div>
    <ol v-if="!recovering" class="switch-steps" aria-label="切换进度">
      <li
        v-for="(step, index) in steps"
        :key="step"
        :class="{ current: step === stage, done: index < steps.indexOf(stage) }"
        :aria-current="step === stage ? 'step' : undefined"
      >
        <span>{{ index < steps.indexOf(stage) ? '✓' : index + 1 }}</span
        >{{ stageLabels[step] }}
      </li>
    </ol>
    <p v-else class="hint">正在停止共享并恢复输出，请稍候；当前系统显示操作结束后会继续恢复。</p>
  </section>
  <section
    v-else-if="state.localOutputRecoveryPending"
    class="panel output-recovery output-failure"
    role="status"
    aria-live="polite"
  >
    <h2>本机显示输出尚未恢复</h2>
    <p>
      {{ state.running ? '将停止共享并重试恢复本机输出。' : '共享已停止，仍可重试恢复本机输出。' }}
      无需连接另一台电脑；若画面仍未显示，请使用显示器实体按键检查输入。
    </p>
    <button
      class="text-button"
      :disabled="busy || state.switching || recovering"
      @click="$emit('action', { type: 'stop' })"
    >
      恢复本机显示输出
    </button>
  </section>
  <section
    v-else-if="completed"
    :class="['output-recovery', { panel: failed, 'output-failure': failed }]"
    role="status"
  >
    <template v-if="failed">
      <h2>输出恢复尚未完成</h2>
      <p>部分输出未能恢复，请重试恢复输出，或使用显示器实体按键检查画面。</p>
    </template>
    <div class="recovery-row">
      <span v-if="!failed" class="completion-label">
        <AppIcon name="tick" :size="16" />
        <span class="completion-text" :title="deviceName(state, completed.target)">
          最近切换至 {{ deviceName(state, completed.target) }}
        </span>
      </span>
      <div class="recovery-actions">
        <button
          class="text-button"
          :disabled="disabled"
          @click="$emit('action', { type: 'switch', target })"
        >
          重试切换
        </button>
        <button
          class="text-button"
          :disabled="busy || state.switching || !state.connected"
          @click="$emit('action', { type: 'restoreOutput', token: completed.token })"
        >
          恢复输出
        </button>
      </div>
    </div>
    <details v-if="!failed" class="completion-details">
      <summary>说明</summary>
      <p>
        输出与键鼠切换流程已完成，无需手动确认。软件无法直接读取实际输入源；若画面未显示，可重试或恢复输出。
      </p>
    </details>
  </section>
</template>

<style scoped>
.progress-heading,
.completion-label {
  display: flex;
  align-items: center;
  gap: 10px;
}
.progress-heading {
  justify-content: space-between;
  flex-wrap: wrap;
}
.output-recovery {
  position: relative;
  margin-bottom: 20px;
  padding: 12px 16px;
  border: 1px solid #dde7db;
  border-radius: 8px;
  background: #f4f8f2;
  font-size: 12px;
}
.recovery-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 24px;
  gap: 16px;
  padding-right: 56px;
}
.recovery-actions {
  display: flex;
  align-items: center;
  flex-shrink: 0;
  gap: 16px;
}
.recovery-actions .text-button {
  white-space: nowrap;
}
.output-failure {
  padding: 20px;
  border-color: #e5cba0;
  background: #fffaf0;
}
.output-failure .recovery-row {
  padding-right: 0;
}
.output-failure p {
  line-height: 1.7;
}
.completion-label {
  flex: 1;
  min-width: 0;
  color: #46663f;
}
.completion-label > svg {
  flex-shrink: 0;
}
.completion-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.completion-details summary {
  position: absolute;
  top: 12px;
  right: 16px;
  padding: 4px 0;
  color: #64806a;
  font-size: 11px;
  line-height: 16px;
  white-space: nowrap;
  cursor: pointer;
}
.completion-details summary:focus-visible {
  outline: 2px solid #6e925f;
  outline-offset: 3px;
  border-radius: 3px;
}
.completion-details p {
  padding: 12px 0 0;
  margin: 12px 0 0;
  border-top: 1px solid #dde7db;
  color: #66765f;
  line-height: 1.7;
}
@media (max-width: 760px) {
  .recovery-row {
    align-items: flex-start;
    flex-direction: column;
    gap: 8px;
  }
  .completion-label {
    flex: none;
    width: 100%;
    min-height: 24px;
  }
  .recovery-actions {
    flex-wrap: wrap;
  }
}
.switch-steps {
  display: flex;
  flex-wrap: wrap;
  gap: 12px 22px;
  padding: 0;
  margin: 16px 0 0;
  list-style: none;
}
.switch-steps li {
  display: flex;
  align-items: center;
  gap: 6px;
  color: #858c82;
  font-size: 12px;
}
.switch-steps li span {
  display: grid;
  place-items: center;
  width: 22px;
  height: 22px;
  border: 1px solid #dfe5dc;
  border-radius: 50%;
}
.switch-steps .current {
  color: #3d6234;
  font-weight: 600;
}
.switch-steps .current span {
  border-color: #6e925f;
  background: #edf5e8;
}
.switch-steps .done {
  color: #58764b;
}
</style>
