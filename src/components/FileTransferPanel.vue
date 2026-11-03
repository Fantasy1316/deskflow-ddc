<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import FileTransferTask from './FileTransferTask.vue'
import FileTransferDropzone from './FileTransferDropzone.vue'
import type { FileAction, FileTransfer, FileTransferNetwork } from '../shared/transfers'
const props = defineProps<{
  transfers: FileTransfer[]
  busy: boolean
  error: string
  warnings: string[]
  speeds: Record<string, number>
  connected: boolean
  supported: boolean
  preview: boolean
  network?: FileTransferNetwork | null
}>()
defineEmits<{ action: [action: FileAction] }>()
const protectingInput = computed(
  () =>
    props.network?.sendLimitBytesPerSecond != null &&
    props.transfers.some((job) => job.direction === 'send' && job.status === 'transferring'),
)
</script>
<template>
  <section>
    <div class="page-heading">
      <div>
        <h1>文件与目录，直接送达。</h1>
        <p>两端配对连接后选择内容，对端确认保存位置后开始接收。</p>
      </div>
    </div>
    <div class="panel transfer-intro">
      <div class="panel-heading">
        <span class="section-icon"><AppIcon name="folder" /></span>
        <div>
          <h2>发送文件或整个目录</h2>
          <p>支持大文件、压缩包、子目录和空目录。文件内容在两台电脑间加密直传。</p>
        </div>
      </div>
      <FileTransferDropzone
        :disabled="busy || !connected || !supported || preview"
        :busy="busy"
        @action="$emit('action', $event)"
      />
      <p v-if="protectingInput" class="hint" role="status">
        键鼠优先：检测到连接延迟升高，正在降低文件发送速率；延迟恢复后将逐步提速。
      </p>
      <p class="hint">
        {{
          preview
            ? '请在桌面应用中使用文件传输。'
            : !connected
              ? '请先在“设备与配对”中连接另一台电脑。'
              : !supported
                ? '对端尚不支持文件传输，请更新两端应用。'
                : '按原名直接保存到所选位置；同名时可选择替换、保留两者或停止。'
        }}
      </p>
    </div>
    <div v-if="error" class="panel transfer-error copyable-text" role="alert">{{ error }}</div>
    <div
      v-for="warning in warnings"
      :key="warning"
      class="panel transfer-error copyable-text"
      role="alert"
    >
      {{ warning }}
    </div>
    <FileTransferTask
      v-for="job in transfers"
      :key="job.id"
      :job="job"
      :busy="busy"
      :speed="speeds[job.id] || 0"
      :connected="connected"
      @action="$emit('action', $event)"
    />
    <div v-if="!transfers.length" class="panel empty-state">
      <AppIcon name="folder" :size="32" />
      <p>还没有文件传输任务</p>
      <small>选择要发送的文件或目录，接收端将在这里看到邀请。</small>
    </div>
    <p class="hint transfer-help">
      暂停或断线会保留已接收部分。应用重启后，由发送端重新发送原任务，再在接收端继续接收。取消接收会清理未完成内容，已完成文件保留。第一版暂不支持符号链接、系统文件剪贴板和跨屏文件拖拽。
    </p>
  </section>
</template>
<style scoped>
.transfer-error {
  color: #a15d45;
  overflow-wrap: anywhere;
}
.transfer-help {
  margin: 0 6px 16px;
}
</style>
