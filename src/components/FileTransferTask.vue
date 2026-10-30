<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import {
  formatBytes,
  transferActive,
  transferLabels,
  transferProgress,
  type FileAction,
  type FileTransfer,
} from '../shared/transfers'
const props = defineProps<{ job: FileTransfer; busy: boolean; speed: number; connected: boolean }>()
defineEmits<{ action: [action: FileAction] }>()
const active = computed(() => transferActive(props.job))
const progress = computed(() => transferProgress(props.job))
function conflictAction(decision: 'replace' | 'keepBoth' | 'stop'): FileAction {
  return { type: 'resolveConflict', id: props.job.id, token: props.job.conflict!.token, decision }
}
</script>
<template>
  <article class="panel transfer-task">
    <div class="task-heading">
      <span class="section-icon"
        ><AppIcon :name="job.direction === 'send' ? 'arrow' : 'download'"
      /></span>
      <div class="task-title">
        <h2>{{ job.label }}</h2>
        <p>
          {{ job.direction === 'send' ? '发送至' : '来自' }} {{ job.peerName }} ·
          {{ job.entries }} 项 · {{ formatBytes(job.totalBytes) }}
        </p>
      </div>
      <span :class="['badge', job.status === 'complete' ? 'success' : 'neutral']">{{
        transferLabels[job.status]
      }}</span>
    </div>
    <template v-if="!['pending', 'cancelled'].includes(job.status)">
      <progress :value="progress" max="100" aria-label="文件传输进度"></progress>
      <div class="task-progress">
        <span
          >已处理 {{ formatBytes(job.completedBytes) }} / {{ formatBytes(job.totalBytes) }}</span
        >
        <span>{{ speed > 0 ? `${formatBytes(Math.round(speed))}/s · ` : '' }}{{ progress }}%</span>
      </div>
    </template>
    <p v-if="job.currentFile && active" class="file-path copyable-text">{{ job.currentFile }}</p>
    <p v-if="job.destination" class="file-path copyable-text">保存位置：{{ job.destination }}</p>
    <p v-if="job.error" class="task-error copyable-text" role="status">{{ job.error }}</p>
    <div
      v-if="job.status === 'conflict' && job.conflict"
      class="conflict-choice"
      role="alertdialog"
      :aria-labelledby="`conflict-${job.id}`"
    >
      <h3 :id="`conflict-${job.id}`">
        目标位置已有同名{{ job.conflict.existingDirectory ? '目录' : '文件' }}
      </h3>
      <p class="copyable-text">{{ job.conflict.name }}</p>
      <p>
        {{
          job.conflict.existingDirectory
            ? '替换将移除原目录及其全部内容，使用收到的项目替代，不会合并目录。'
            : '替换将使用收到的项目覆盖原文件。'
        }}保留两者会为收到的项目添加编号。
      </p>
      <p>停止操作将保留传输缓存，原有文件和目录不变。</p>
      <div class="task-actions">
        <button
          class="button secondary compact"
          :disabled="busy"
          @click="$emit('action', conflictAction('replace'))"
        >
          替换
        </button>
        <button
          class="button primary compact"
          :disabled="busy"
          @click="$emit('action', conflictAction('keepBoth'))"
        >
          保留两者
        </button>
        <button
          class="text-button"
          :disabled="busy"
          @click="$emit('action', conflictAction('stop'))"
        >
          停止操作
        </button>
      </div>
    </div>
    <div class="task-actions">
      <button
        v-if="job.direction === 'receive' && ['pending', 'paused'].includes(job.status)"
        class="button primary compact"
        :disabled="busy"
        @click="$emit('action', { type: 'accept', id: job.id })"
      >
        {{ job.completedBytes ? '继续接收' : '接收并保存' }}
      </button>
      <button
        v-if="job.direction === 'send' && ['paused', 'waiting'].includes(job.status)"
        class="button primary compact"
        :disabled="busy || !connected"
        @click="$emit('action', { type: 'resume', id: job.id })"
      >
        重新发送此任务
      </button>
      <button
        v-if="active"
        class="button secondary compact"
        :disabled="busy"
        @click="$emit('action', { type: 'pause', id: job.id })"
      >
        暂停
      </button>
      <button
        v-if="job.direction === 'receive' && job.status === 'complete'"
        class="button secondary compact"
        :disabled="busy"
        @click="$emit('action', { type: 'open', id: job.id })"
      >
        <AppIcon name="folder" :size="15" />打开接收目录
      </button>
      <button
        v-if="!['complete', 'cancelled'].includes(job.status)"
        class="text-button"
        :disabled="busy"
        @click="$emit('action', { type: 'cancel', id: job.id })"
      >
        {{ job.direction === 'receive' && job.status === 'pending' ? '拒绝接收' : '取消任务' }}
      </button>
      <button
        v-if="['complete', 'cancelled'].includes(job.status)"
        class="text-button"
        :disabled="busy"
        @click="$emit('action', { type: 'remove', id: job.id })"
      >
        移除记录
      </button>
    </div>
  </article>
</template>
<style scoped>
.task-heading {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  margin-bottom: 18px;
}
.task-title {
  flex: 1;
  min-width: 0;
}
.task-title h2 {
  overflow-wrap: anywhere;
}
.task-title p,
.task-progress,
.file-path {
  color: var(--muted);
  font-size: 12px;
}
.task-title p {
  margin-top: 5px;
}
progress {
  display: block;
  width: 100%;
  height: 7px;
  border: 0;
  border-radius: 5px;
  overflow: hidden;
  accent-color: var(--green);
}
progress::-webkit-progress-bar {
  background: #eef1ef;
}
progress::-webkit-progress-value {
  background: var(--green);
  border-radius: 5px;
}
.task-progress {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  margin-top: 8px;
}
.file-path {
  margin-top: 12px;
  overflow-wrap: anywhere;
}
.task-error {
  color: #a15d45;
  font-size: 12px;
  margin-top: 12px;
  overflow-wrap: anywhere;
}
.task-actions {
  display: flex;
  gap: 14px;
  align-items: center;
  flex-wrap: wrap;
  margin-top: 18px;
}
.conflict-choice {
  margin-top: 16px;
  padding: 16px;
  background: var(--surface, #f5f7f5);
  border: 1px solid var(--line, #dce3df);
  border-radius: 10px;
}
.conflict-choice h3 {
  font-size: 14px;
}
.conflict-choice p {
  margin-top: 8px;
  font-size: 12px;
  line-height: 1.7;
  overflow-wrap: anywhere;
}
</style>
