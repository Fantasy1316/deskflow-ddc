<script setup lang="ts">
import { computed, useTemplateRef } from 'vue'
import AppIcon from './AppIcon.vue'
import FileSendStatus from './FileSendStatus.vue'
import appIcon from '../assets/app-icon.png'
import { useFileSendPanel } from '../composables/useFileSendPanel'
import { useFileDrop } from '../composables/useFileDrop'
import { usePanelSize } from '../composables/usePanelSize'

const { state, disabled, pending, error, choose, close, showMain, retry, resize } =
  useFileSendPanel()
const panel = useTemplateRef<HTMLElement>('panel')
const { dragging: nativeDragging, error: dropError } = useFileDrop(
  () => panel.value,
  () => disabled.value,
  // The OS window handles drops even if this WebView is still loading.
  () => {},
  'file-send',
)
usePanelSize(() => panel.value, resize)
const dragging = computed(() => !disabled.value && (nativeDragging.value || state.value.dragging))
const busy = computed(() => pending.value || state.value.busy)
const message = computed(
  () =>
    error.value ||
    dropError.value ||
    state.value.disabledReason ||
    (busy.value && state.value.scannedEntries !== null
      ? `正在整理，已发现 ${state.value.scannedEntries} 项文件与目录`
      : '') ||
    state.value.message ||
    (state.value.activity ? '' : '松手即发送，关闭面板后传输仍会继续'),
)
const failed = computed(() => !!error.value || !!dropError.value || state.value.error)
const macos =
  typeof window !== 'undefined' &&
  new URLSearchParams(window.location.search).get('platform') === 'macos'
</script>

<template>
  <section
    ref="panel"
    class="mini-send"
    :class="{ macos, dragging }"
    aria-label="迷你文件发送面板"
    @keydown.esc="close"
  >
    <header class="mini-header">
      <div class="mini-brand">
        <img :src="appIcon" alt="" width="23" height="23" /><span>Deskflow DDC</span>
      </div>
      <button class="mini-close" title="关闭发送面板" aria-label="关闭发送面板" @click="close">
        <AppIcon name="close" :size="17" />
      </button>
    </header>
    <div class="mini-target">
      <h1>发送文件</h1>
      <span class="mini-peer" :title="state.peerName || '尚未连接设备'"
        ><span class="mini-dot" :class="{ ready: !state.disabledReason }" /><span
          class="mini-peer-name"
          >{{ state.peerName ? `发送至 ${state.peerName}` : '尚未连接设备' }}</span
        ></span
      >
    </div>
    <button
      class="mini-drop"
      :class="{ dragging }"
      :disabled="disabled"
      :aria-busy="busy"
      @click="choose()"
    >
      <span class="mini-upload"><AppIcon :name="busy ? 'clock' : 'upload'" :size="27" /></span>
      <strong>{{ busy ? '正在准备文件…' : dragging ? '松开即可发送' : '将文件拖到这里' }}</strong>
      <small>{{ busy ? '可关闭面板，任务会在后台继续' : '或点击选择文件' }}</small>
    </button>
    <div class="mini-details">
      <span>支持文件、文件夹与多选</span>
      <button class="text-button" :disabled="disabled" @click="choose(true)">选择文件夹</button>
    </div>
    <FileSendStatus
      :activity="busy ? null : state.activity"
      :message="message"
      :error="failed"
      :disabled="disabled"
      @retry="retry"
      @show-transfers="showMain"
    />
    <footer class="mini-footer">
      <span><AppIcon name="shield" :size="12" /> 局域网加密传输</span>
      <button class="text-button" @click="showMain">
        {{ state.activity ? '查看传输' : '打开主界面' }}<AppIcon name="chevron" :size="13" />
      </button>
    </footer>
  </section>
</template>

<style scoped>
.mini-send {
  width: 100%;
  display: flex;
  flex-direction: column;
  padding: 18px 20px 16px;
  border: 1px solid #dfe7df;
  background: linear-gradient(150deg, #fff 10%, #f5f8f3);
  color: var(--ink);
}
.mini-send.dragging {
  box-shadow: inset 0 0 0 2px #8bad8b;
  background: linear-gradient(150deg, #f7fbf4, #eef5e9);
}
.mini-send.macos {
  padding-top: 28px;
  border: 0;
}
.mini-header,
.mini-brand,
.mini-target,
.mini-details,
.mini-footer {
  display: flex;
  align-items: center;
}
.mini-header {
  justify-content: space-between;
}
.mini-brand {
  gap: 7px;
  font-size: 11px;
  font-weight: 600;
  color: #6a7c71;
}
.mini-brand img {
  border-radius: 6px;
}
.mini-close {
  display: grid;
  place-items: center;
  width: 25px;
  height: 25px;
  padding: 0;
  border: 0;
  border-radius: 7px;
  color: #819087;
  background: transparent;
}
.mini-close:hover {
  background: #e8eee6;
}
.mini-target {
  margin: 16px 0 13px;
  justify-content: space-between;
  gap: 14px;
}
.mini-target h1 {
  flex-shrink: 0;
  font-size: 18px;
  letter-spacing: -0.5px;
}
.mini-peer {
  display: flex;
  align-items: center;
  gap: 5px;
  color: #718277;
  font-size: 11px;
  min-width: 0;
}
.mini-peer-name {
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.mini-dot {
  display: inline-block;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: #a9b3ad;
  flex-shrink: 0;
}
.mini-dot.ready {
  background: #649271;
}
.mini-drop {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  gap: 5px;
  width: 100%;
  height: 134px;
  border: 1px dashed #bbcfbb;
  border-radius: 12px;
  background: #f0f5ed;
  color: #41664d;
  transition:
    background 0.15s,
    border-color 0.15s;
}
.mini-drop:hover:not(:disabled),
.mini-drop.dragging {
  background: #e4efdf;
  border-color: #688e66;
}
.mini-drop:disabled {
  opacity: 0.62;
}
.mini-upload {
  display: grid;
  place-items: center;
  margin-bottom: 4px;
}
.mini-drop strong {
  font-size: 13px;
  font-weight: 600;
}
.mini-drop small {
  font-size: 11px;
  color: #7c8e7c;
}
.mini-details {
  justify-content: space-between;
  margin-top: 10px;
  gap: 8px;
  font-size: 10px;
  color: #87938a;
}
.mini-details .text-button {
  font-size: 11px;
}
.mini-footer {
  flex-shrink: 0;
  padding-top: 11px;
  border-top: 1px solid #e4eae1;
  justify-content: space-between;
  gap: 10px;
  color: #8a978b;
  font-size: 10px;
}
.mini-footer span {
  display: flex;
  align-items: center;
  gap: 4px;
}
.mini-footer .text-button {
  font-size: 11px;
}
</style>
