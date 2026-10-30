<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import { actionLabels, type ActionFailure } from '../shared/workspace'
const props = defineProps<{
  failure: ActionFailure
  busy: boolean
  running: boolean
  preview: boolean
  displayOnly?: boolean
}>()
defineEmits<{ dismiss: []; export: []; stop: []; navigate: [page: string] }>()
const stage = computed(() =>
  props.displayOnly
    ? props.failure.action === 'start'
      ? '启用显示器切换'
      : props.failure.action === 'switch'
        ? '切换显示器输入'
        : actionLabels[props.failure.action] || '运行显示器切换'
    : actionLabels[props.failure.action] || '运行共享功能',
)
const destination = computed(() =>
  props.displayOnly || ['switch', 'testInput', 'scan'].includes(props.failure.action)
    ? 'display'
    : 'pairing',
)
</script>
<template>
  <section class="runtime-error" aria-labelledby="error-title" role="alert">
    <AppIcon name="alert" :size="21" />
    <div class="error-body">
      <h2 id="error-title">{{ stage }}遇到问题</h2>
      <p>操作未完整完成，部分状态可能已改变。请检查配置与连接，保留日志以便排查。</p>
      <details>
        <summary>查看错误详情</summary>
        <pre class="copyable-text">{{ failure.message }}</pre>
      </details>
      <div class="button-group">
        <button
          class="button secondary compact"
          :disabled="busy || preview"
          @click="$emit('export')"
        >
          导出诊断日志</button
        ><button class="text-button" @click="$emit('navigate', destination)">
          检查{{ destination === 'display' ? '显示器配置' : '设备连接' }}</button
        ><button v-if="running" class="text-button" :disabled="busy" @click="$emit('stop')">
          {{ displayOnly ? '停用显示器切换' : '停止共享' }}</button
        ><button class="text-button" @click="$emit('navigate', 'activity')">查看活动记录</button>
      </div>
    </div>
    <button class="icon-button dismiss-error" aria-label="关闭错误提示" @click="$emit('dismiss')">
      <AppIcon name="close" :size="18" />
    </button>
  </section>
</template>
