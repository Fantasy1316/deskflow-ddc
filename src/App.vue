<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import AppIcon from './components/AppIcon.vue'
import appIcon from './assets/app-icon.png'
import Dashboard from './components/Dashboard.vue'
import RuntimeError from './components/RuntimeError.vue'
import RecoveryHelp from './components/RecoveryHelp.vue'
import PairingPanel from './components/PairingPanel.vue'
import DisplayPanel from './components/DisplayPanel.vue'
import SharingPanel from './components/SharingPanel.vue'
import DisplayOnlySettings from './components/DisplayOnlySettings.vue'
import ActivityPanel from './components/ActivityPanel.vue'
import FileTransferPanel from './components/FileTransferPanel.vue'
import { useTransfers } from './composables/useTransfers'
import { useBridge } from './composables/useBridge'
import { version } from '../package.json'
const {
  state,
  loading,
  busy,
  pendingAction,
  notice,
  noticeError,
  displayConfigured,
  failure,
  dismissFailure,
  act,
  saveAndStart,
} = useBridge()
const page = shallowRef('dashboard')
const pageScroll = ref<HTMLDivElement | null>(null)
watch(
  page,
  () => {
    pageScroll.value?.scrollTo({ top: 0 })
  },
  { flush: 'post' },
)
const files = useTransfers()
const allNav = [
  { id: 'dashboard', label: '工作台', icon: 'dashboard' },
  { id: 'pairing', label: '设备与配对', icon: 'link' },
  { id: 'display', label: '显示器与输入', icon: 'monitor' },
  { id: 'sharing', label: '共享设置', icon: 'settings' },
  { id: 'files', label: '文件传输', icon: 'folder' },
  { id: 'activity', label: '活动记录', icon: 'clock' },
] as const
const displayOnly = computed(() => state.value.config.mode === 'display-only')
const nav = computed(() =>
  allNav
    .filter((item) => !displayOnly.value || !['pairing', 'files'].includes(item.id))
    .map((item) => ({
      ...item,
      label: displayOnly.value && item.id === 'sharing' ? '切换设置' : item.label,
    })),
)
watch(displayOnly, () => {
  if (displayOnly.value && ['pairing', 'files'].includes(page.value)) page.value = 'dashboard'
})
const title = computed(() => nav.value.find((item) => item.id === page.value)?.label ?? '工作台')
const connectionLabel = computed(() =>
  displayOnly.value
    ? state.value.running
      ? '显示器切换已启用'
      : '仅切换显示器 · 未启用'
    : state.value.engine === 'error'
      ? '引擎连接失败'
      : state.value.engine === 'ready'
        ? '共享已连接'
        : state.value.connected
          ? '引擎连接中'
          : state.value.running
            ? '等待连接'
            : '尚未连接',
)
const on = computed(() =>
  displayOnly.value ? state.value.running : state.value.connected && state.value.engine === 'ready',
)
</script>
<template>
  <div class="app-layout">
    <aside class="sidebar">
      <div class="brand">
        <span class="brand-mark"
          ><img :src="appIcon" width="36" height="36" alt="Deskflow DDC"
        /></span>
        <div><b>Deskflow DDC</b><small>一屏双机 · 自由切换</small></div>
      </div>
      <span class="nav-caption">工作空间</span>
      <nav class="nav-list" aria-label="主导航">
        <button
          v-for="item in nav"
          :key="item.id"
          :class="['nav-item', { selected: page === item.id }]"
          :aria-current="page === item.id ? 'page' : undefined"
          @click="page = item.id"
        >
          <AppIcon :name="item.icon" /><span>{{ item.label }}</span
          ><span v-if="item.id === 'activity' && state.lastError" class="nav-alert"></span>
          <span
            v-if="
              item.id === 'files' && files.transfers.value.some((job) => job.status === 'pending')
            "
            class="nav-alert"
          ></span>
        </button>
      </nav>
      <div class="sidebar-bottom">
        <div class="local-device">
          <span class="device-avatar"><AppIcon name="laptop" :size="20" /></span>
          <div>
            <strong>{{ state.config.deviceName }}</strong
            ><small
              >{{
                state.platform === 'darwin'
                  ? 'macOS'
                  : state.platform === 'win32'
                    ? 'Windows'
                    : '浏览器预览'
              }}
              · 本机</small
            >
          </div>
        </div>
        <div class="version">
          <span>Deskflow DDC</span><span>v{{ version }}</span>
        </div>
      </div>
    </aside>
    <div class="main-area">
      <header class="topbar">
        <div class="breadcrumb">
          <strong>{{ title }}</strong>
        </div>
        <div class="topbar-actions">
          <span :class="['status-pill', { online: on }]"><i></i>{{ connectionLabel }}</span>
          <button
            v-if="state.running"
            class="button secondary compact stop-sharing"
            :disabled="busy"
            @click="act({ type: 'stop' })"
          >
            <AppIcon name="power" :size="16" />{{ displayOnly ? '停用切换' : '停止共享' }}
          </button>
          <RecoveryHelp
            :display-only="displayOnly"
            :running="state.running"
            :busy="busy"
            @stop="act({ type: 'stop' })"
          />
        </div>
      </header>
      <div ref="pageScroll" class="page-scroll">
        <div v-if="state.preview" class="preview-strip">
          <AppIcon name="monitor" :size="15" /> 浏览器预览 · 显示器、网络与键鼠操作需要打开桌面 App
        </div>
        <main class="content">
          <RuntimeError
            v-if="failure"
            :failure="failure"
            :display-only="displayOnly"
            :busy="busy"
            :running="state.running"
            :preview="state.preview"
            @dismiss="dismissFailure"
            @export="act({ type: 'exportDiagnostics' })"
            @stop="act({ type: 'stop' })"
            @navigate="page = $event"
          />
          <div v-if="loading" class="empty-state">正在读取设备状态…</div>
          <Dashboard
            v-else-if="page === 'dashboard'"
            :state="state"
            :busy="busy"
            :display-configured="displayConfigured"
            :execute="act"
            @action="act"
            @navigate="page = $event"
            @switch="act({ type: 'switch', target: $event })"
            @lock-peer="act({ type: 'lockPeer' })"
          />
          <PairingPanel
            v-else-if="page === 'pairing'"
            :state="state"
            :busy="busy"
            :connect="saveAndStart"
            @action="act"
          />
          <DisplayPanel
            v-else-if="page === 'display'"
            :state="state"
            :busy="busy"
            :execute="act"
            @action="act"
          />
          <DisplayOnlySettings
            v-else-if="page === 'sharing' && displayOnly"
            :state="state"
            :busy="busy"
            @action="act"
          />
          <SharingPanel
            v-else-if="page === 'sharing'"
            :state="state"
            :busy="busy"
            :checking-accessibility="pendingAction === 'checkAccessibility'"
            @action="act"
          />
          <ActivityPanel
            v-else-if="page === 'activity'"
            :logs="state.logs"
            :busy="busy"
            :storage-error="state.diagnostics.error"
            :preview="state.preview"
            @export="act({ type: 'exportDiagnostics' })"
            @open-folder="act({ type: 'openLogFolder' })"
            @open-config="act({ type: 'openDataFolder' })"
          />
          <FileTransferPanel
            v-else
            :transfers="files.transfers.value"
            :busy="files.busy.value"
            :error="files.error.value"
            :warnings="files.warnings.value"
            :speeds="files.speeds.value"
            :network="files.network.value"
            :connected="state.connected"
            :supported="state.peer?.fileTransfer ?? false"
            :preview="state.preview"
            @action="files.act"
          />
        </main>
        <footer class="app-footer">
          <span
            ><AppIcon name="shield" :size="14" />
            {{
              displayOnly ? '本机 DDC 控制 · 无需网络连接' : '局域网直连 · 数据留在你的设备之间'
            }}</span
          ><span>一个工作空间，两台电脑</span>
        </footer>
      </div>
    </div>
    <div v-if="notice" :class="['toast', { error: noticeError }]" role="status">
      <AppIcon :name="noticeError ? 'alert' : 'check'" :size="20" />
      <span :class="{ 'copyable-text': noticeError }">{{ notice }}</span>
      <button aria-label="关闭提示" @click="notice = ''">
        <AppIcon name="close" :size="16" />
      </button>
    </div>
  </div>
</template>
