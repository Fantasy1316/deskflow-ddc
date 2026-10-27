<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import type { Snapshot } from '../shared/types'
const props = defineProps<{ state: Snapshot; busy: boolean; displayConfigured: boolean }>()
defineEmits<{ navigate: [page: string]; switch: [target: 'local' | 'peer']; stop: [] }>()
const monitorName = computed(
  () =>
    props.state.monitors.find((m) => m.id === props.state.config.displayId)?.name ||
    (props.state.config.displayId ? '已配置的显示器' : '等待选择显示器'),
)
const ready = computed(() => props.state.engine === 'ready' && props.state.connected)
const videoMode = computed(() =>
  props.state.config.role === 'host'
    ? props.state.config.crossScreenVideo
    : (props.state.sessionLayout?.video ?? 'manual'),
)
const videoLabel = computed(() =>
  videoMode.value === 'manual'
    ? '画面仅手动切换'
    : videoMode.value === 'keep'
      ? '按屏幕配置跨屏'
      : '画面同步切换',
)
const activeName = computed(() =>
  !props.state.activeDeviceId
    ? '尚未开始共享'
    : props.state.activeDeviceId === props.state.deviceId
      ? props.state.config.deviceName
      : props.state.peer?.name || '另一台电脑',
)
const displayName = computed(() =>
  props.state.displayStatus === 'uncertain'
    ? '输入状态不确定，请停止共享并手动确认画面'
    : !props.state.displayDeviceId
      ? '外接屏输入以实际画面为准'
      : props.state.displayDeviceId === props.state.deviceId
        ? props.state.config.deviceName
        : props.state.peer?.name || '另一台电脑',
)
const steps = computed(() => [
  {
    title: '配置共享显示器',
    text: '选择显示器，设置两台电脑的输入源',
    done: props.displayConfigured,
    page: 'display',
    icon: 'monitor' as const,
  },
  {
    title: '配对另一台电脑',
    text: '选择键鼠角色，建立局域网连接',
    done: props.state.connected,
    page: 'pairing',
    icon: 'link' as const,
  },
  {
    title: '开始你的无缝工作',
    text: '用快捷键一起切换画面和键鼠',
    done: ready.value,
    page: 'sharing',
    icon: 'switch' as const,
  },
])
const shortcut = computed(() =>
  props.state.config.shortcut
    .replace('control', 'Ctrl')
    .replace('alt', 'Alt')
    .replace('shift', 'Shift')
    .split('+'),
)
</script>
<template>
  <section class="dashboard-page">
    <div class="page-heading">
      <div>
        <span class="eyebrow">YOUR CONNECTED WORKSPACE</span>
        <h1>一块屏幕，两台电脑。</h1>
        <p>切换设备，让思路继续。</p>
      </div>
      <button class="button secondary" @click="$emit('navigate', 'pairing')">
        <AppIcon name="plus" :size="17" />配置设备
      </button>
    </div>
    <div class="workspace-card">
      <div class="workspace-caption">
        <span class="subtle-label"><i class="tiny-dot"></i>共享工作空间</span
        ><span class="light-badge">{{ ready ? '共享运行中' : '等待建立连接' }}</span>
      </div>
      <div class="workspace-visual">
        <div class="computer-node">
          <div class="computer-icon local">
            <AppIcon name="laptop" :size="34" /><span
              class="node-dot"
              :class="{ on: ready }"
            ></span>
          </div>
          <strong>{{ state.config.deviceName }}</strong
          ><small>本机 · {{ state.config.role === 'host' ? '提供键鼠' : '接收键鼠' }}</small
          ><span class="input-badge">输入源 {{ state.config.localInput ?? '待配置' }}</span>
        </div>
        <div class="connection-line">
          <span></span><AppIcon name="switch" :size="18" /><span></span>
        </div>
        <div class="shared-display">
          <div class="display-bezel">
            <div class="display-surface">
              <span class="surface-orbit orbit-one"></span
              ><span class="surface-orbit orbit-two"></span
              ><span class="surface-orbit orbit-three"></span>
              <div class="surface-label">
                <AppIcon name="monitor" :size="26" /><strong>{{ monitorName }}</strong
                ><small>{{ ready ? displayName : '让工作空间连接起来' }}</small>
              </div>
            </div>
            <i></i>
          </div>
          <div class="display-stand"></div>
          <div class="display-foot"></div>
          <span class="display-caption">同一块显示器</span>
        </div>
        <div class="connection-line">
          <span></span><AppIcon name="switch" :size="18" /><span></span>
        </div>
        <div class="computer-node">
          <div class="computer-icon">
            <AppIcon name="laptop" :size="34" /><span
              class="node-dot"
              :class="{ on: state.connected }"
            ></span>
          </div>
          <strong>{{ state.peer?.name || '另一台电脑' }}</strong
          ><small
            >{{ state.connected ? '已配对' : '等待配对' }} ·
            {{ state.config.role === 'host' ? '接收键鼠' : '提供键鼠' }}</small
          ><span class="input-badge">输入源 {{ state.config.peerInput ?? '待配置' }}</span>
        </div>
      </div>
      <div class="workspace-controls">
        <div class="active-label">
          <span class="pulse-dot" :class="{ on: ready }"></span
          ><span>{{ state.switching ? '正在协调切换…' : '键鼠：' + activeName }}</span>
        </div>
        <div class="button-group">
          <button
            class="button secondary compact"
            :disabled="!ready || busy || state.switching"
            @click="$emit('switch', 'local')"
          >
            切到本机</button
          ><button
            class="button primary compact"
            :disabled="!ready || busy || state.switching"
            @click="$emit('switch', 'peer')"
          >
            <AppIcon name="switch" :size="16" />切到另一台
          </button>
        </div>
      </div>
    </div>
    <div class="feature-grid">
      <div class="feature-card">
        <span class="feature-icon"><AppIcon name="keyboard" /></span>
        <div>
          <strong>一套键盘鼠标</strong>
          <p>{{ ready ? '共享引擎已连接' : '连接后，一套键鼠控制两端' }}</p>
        </div>
        <i :class="['feature-indicator', { on: ready }]"></i>
      </div>
      <div class="feature-card">
        <span class="feature-icon"><AppIcon name="clipboard" /></span>
        <div>
          <strong>共享剪贴板</strong>
          <p>
            {{
              !state.config.clipboard
                ? '已关闭'
                : ready
                  ? '已随键鼠共享启用'
                  : '复制与粘贴，跨越设备'
            }}
          </p>
        </div>
        <i :class="['feature-indicator', { on: ready && state.config.clipboard }]"></i>
      </div>
      <div class="feature-card">
        <span class="feature-icon"><AppIcon name="monitor" /></span>
        <div>
          <strong>{{ videoLabel }}</strong>
          <p>
            {{
              videoMode === 'manual'
                ? '快捷键或按钮切画面，边缘只转移键鼠'
                : displayConfigured
                  ? '输入源映射已配置'
                  : '切换电脑，也切换显示器'
            }}
          </p>
        </div>
        <i :class="['feature-indicator', { on: displayConfigured }]"></i>
      </div>
    </div>
    <div v-if="!ready" class="section-heading">
      <h2>三步，连接你的工作空间</h2>
      <span>首次使用指南</span>
    </div>
    <div v-if="!ready" class="setup-steps">
      <button
        v-for="(step, index) in steps"
        :key="step.title"
        class="setup-step"
        @click="$emit('navigate', step.page)"
      >
        <span :class="['step-number', { done: step.done }]"
          ><AppIcon v-if="step.done" name="tick" :size="16" /><template v-else
            >0{{ index + 1 }}</template
          ></span
        >
        <div>
          <strong>{{ step.title }}</strong>
          <p>{{ step.text }}</p>
        </div>
        <AppIcon name="chevron" :size="18" />
      </button>
    </div>
    <div class="shortcut-strip">
      <span
        ><AppIcon name="keyboard" :size="18" />{{
          state.config.role === 'host' ? '你的切换快捷键' : '切换快捷键由键鼠提供端设置'
        }}</span
      >
      <div class="keys">
        <kbd v-for="key in shortcut" :key="key">{{ key }}</kbd>
      </div>
      <button class="text-button" @click="$emit('navigate', 'sharing')">
        修改设置 <AppIcon name="chevron" :size="14" />
      </button>
    </div>
    <div v-if="ready" class="recovery-note">
      <span>遇到异常时，按 <b>Ctrl + Alt + Esc</b> 尝试恢复到键鼠提供端。</span
      ><button class="text-button" :disabled="busy" @click="$emit('stop')">停止共享</button>
    </div>
  </section>
</template>
