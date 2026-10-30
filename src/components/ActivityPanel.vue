<script setup lang="ts">
import AppIcon from './AppIcon.vue'
import type { LogEntry } from '../shared/types'
defineProps<{ logs: LogEntry[]; busy: boolean; storageError: string | null; preview: boolean }>()
defineEmits<{ export: []; openFolder: []; openConfig: [] }>()
const time = (value: string) =>
  new Intl.DateTimeFormat('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).format(new Date(value))
</script>
<template>
  <section>
    <div class="page-heading activity-heading">
      <div>
        <h1>活动记录</h1>
        <p>出现错误后，导出两台电脑的诊断日志即可协助排查。</p>
      </div>
      <button class="button primary" :disabled="busy || preview" @click="$emit('export')">
        <AppIcon name="download" :size="16" />导出诊断日志
      </button>
    </div>
    <div class="panel log-summary">
      <div>
        <strong>{{
          preview ? '诊断日志' : storageError ? '日志写入失败' : '已开启自动日志记录'
        }}</strong>
        <p>
          {{
            preview
              ? '请在桌面 App 中导出诊断日志。'
              : storageError
                ? '部分记录暂存在内存中，请立即导出；退出后内存记录不会保留。'
                : '日志跨重启保留，最多 5 个文件、每个 2 MB。导出文件包含版本、运行状态与近期日志。'
          }}
        </p>
        <p>包含设备名和局域网地址；不收集键鼠操作或剪贴板内容，配对码与私钥会过滤。</p>
        <p v-if="storageError" class="storage-error copyable-text" role="alert">
          {{ storageError }}
        </p>
      </div>
      <div class="log-actions">
        <button
          class="button secondary compact"
          :disabled="busy || preview"
          @click="$emit('openFolder')"
        >
          <AppIcon name="folder" :size="15" />日志目录
        </button>
        <button class="text-button" :disabled="busy || preview" @click="$emit('openConfig')">
          配置目录<AppIcon name="external" :size="12" />
        </button>
      </div>
    </div>
    <div class="panel">
      <div v-if="!logs.length" class="empty-state">
        <AppIcon name="clock" :size="32" />
        <p>还没有活动记录</p>
        <small>开始配置或共享后，运行结果会显示在这里。</small>
      </div>
      <ol v-else class="activity-list">
        <li v-for="log in logs" :key="log.id" :class="['activity-entry', log.level]">
          <span class="activity-symbol"
            ><AppIcon
              :name="log.level === 'error' ? 'alert' : log.level === 'success' ? 'check' : 'clock'"
              :size="18"
          /></span>
          <p class="copyable-text">{{ log.message }}</p>
          <time :datetime="log.time">{{ time(log.time) }}</time>
        </li>
      </ol>
    </div>
  </section>
</template>

<style scoped>
.activity-heading {
  flex-wrap: wrap;
}
.log-summary {
  display: flex;
  justify-content: space-between;
  gap: 20px;
  align-items: flex-start;
}
.log-summary strong {
  font-size: 12px;
  font-weight: 600;
}
.log-summary p {
  color: #627467;
  font-size: 12px;
  line-height: 1.8;
  margin-top: 7px;
}
.log-summary .storage-error {
  color: #a17d55;
  overflow-wrap: anywhere;
}
.log-actions {
  display: flex;
  align-items: flex-end;
  flex-direction: column;
  gap: 12px;
  flex-shrink: 0;
}
</style>
