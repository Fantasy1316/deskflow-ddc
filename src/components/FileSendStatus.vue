<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import type { FileSendActivity } from '../composables/useFileSendPanel'
import { formatBytes } from '../shared/transfers'

const props = defineProps<{
  activity: FileSendActivity | null
  message: string
  error: boolean
  disabled: boolean
}>()
defineEmits<{ retry: []; showTransfers: [] }>()
const progress = computed(() => {
  const job = props.activity
  return job?.status === 'complete'
    ? 100
    : job?.totalBytes
      ? Math.min(99, Math.floor((job.completedBytes / job.totalBytes) * 100))
      : 0
})
const title = computed(() => {
  const job = props.activity
  if (!job) return ''
  switch (job.status) {
    case 'complete':
      return `${job.items} 项已发送完成`
    case 'waiting':
    case 'pending':
      return `已添加 ${job.items} 项，等待对方接收`
    case 'connecting':
      return '正在建立传输连接'
    case 'transferring':
      return `发送与校验中 · ${progress.value}%`
    case 'paused':
      return '发送已暂停'
    case 'cancelled':
      return '发送已取消'
    case 'conflict':
      return '等待对方处理同名项目'
    default:
      return ''
  }
})
const detail = computed(
  () =>
    (props.message !== title.value ? props.message : '') ||
    props.activity?.error ||
    props.activity?.label ||
    '',
)
</script>

<template>
  <div class="send-status" :class="{ error: error || !!activity?.error }" aria-live="polite">
    <template v-if="activity">
      <div class="status-row">
        <strong>{{ title }}</strong>
        <span v-if="activity.status === 'transferring'" class="status-speed"
          >{{ formatBytes(activity.bytesPerSecond) }}/s</span
        >
        <button
          v-else-if="activity.status === 'paused'"
          class="text-button"
          :disabled="disabled"
          @click="$emit('retry')"
        >
          重试
        </button>
      </div>
      <div
        v-if="activity.status === 'transferring'"
        class="status-progress"
        role="progressbar"
        :aria-valuenow="progress"
        aria-valuemin="0"
        aria-valuemax="100"
        aria-label="最近发送进度"
      >
        <span :style="{ width: `${progress}%` }" />
      </div>
    </template>
    <p v-if="detail" class="status-detail" :title="detail">{{ detail }}</p>
    <button
      v-if="activity?.status === 'conflict'"
      class="text-button"
      @click="$emit('showTransfers')"
    >
      查看传输<AppIcon name="chevron" :size="12" />
    </button>
  </div>
</template>

<style scoped>
.send-status {
  margin: 11px 0 12px;
  color: #718277;
  font-size: 11px;
}
.status-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.status-row strong {
  font-size: 11px;
  font-weight: 600;
}
.status-speed {
  white-space: nowrap;
  color: #87938a;
  font-variant-numeric: tabular-nums;
}
.status-detail {
  margin: 0;
  line-height: 1.6;
  overflow-wrap: anywhere;
  max-height: 53px;
  overflow-y: auto;
}
.status-row + .status-detail {
  margin-top: 3px;
}
.status-progress {
  height: 3px;
  background: #e5ece2;
  border-radius: 3px;
  overflow: hidden;
  margin: 8px 0 5px;
}
.status-progress span {
  display: block;
  height: 100%;
  background: #678c6e;
  transition: width 0.2s;
}
.send-status.error {
  color: #af5748;
}
.send-status .text-button {
  font-size: 11px;
}
</style>
