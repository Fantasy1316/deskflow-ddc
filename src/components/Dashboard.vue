<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import OperationModePicker from './OperationModePicker.vue'
import type { ActionRunner } from '../shared/workspace'
import type { Action } from '../shared/types'
import WorkspaceOverview from './dashboard/WorkspaceOverview.vue'
import type { Snapshot } from '../shared/types'
import { deviceName, pictureCommandLabel } from '../shared/workspace'
const props = defineProps<{
  state: Snapshot
  busy: boolean
  displayConfigured: boolean
  execute: ActionRunner
}>()
defineEmits<{
  navigate: [page: string]
  switch: [target: 'local' | 'peer']
  lockPeer: []
  action: [action: Action]
}>()
const displayOnly = computed(() => props.state.config.mode === 'display-only')
const ready = computed(() =>
  displayOnly.value ? props.state.running : props.state.engine === 'ready' && props.state.connected,
)
const activeName = computed(() =>
  props.state.activeDeviceId
    ? deviceName(props.state, props.state.activeDeviceId)
    : '尚未建立键鼠共享',
)
const picture = computed(() => pictureCommandLabel(props.state))
const steps = computed(() =>
  displayOnly.value
    ? [
        {
          title: '配置显示器',
          text: '保存两个输入源，测试往返切换',
          done: props.displayConfigured,
          page: 'display',
        },
        { title: '设置切换快捷键', text: '配置本机快捷键，无需配对', done: false, page: 'sharing' },
      ]
    : [
        {
          title: '配置显示器',
          text: '选择外接屏，测试两台电脑的输入源',
          done: props.displayConfigured,
          page: 'display',
        },
        {
          title: '配对设备',
          text: '指定键鼠提供端，建立局域网连接',
          done: props.state.connected,
          page: 'pairing',
        },
        {
          title: '检查共享设置',
          text: '查看快捷键与鼠标跨屏方式',
          done: ready.value,
          page: 'sharing',
        },
      ],
)
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
        <h1>工作台</h1>
        <p>
          {{
            displayOnly
              ? '无需网络连接，仅通过本机切换显示器输入源。'
              : '查看设备状态，切换共享显示器和键鼠。'
          }}
        </p>
      </div>
      <button
        class="button secondary"
        @click="$emit('navigate', displayOnly ? 'display' : 'pairing')"
      >
        <AppIcon name="settings" :size="17" />{{ displayOnly ? '配置显示器' : '管理设备' }}
      </button>
    </div>
    <OperationModePicker :state="state" :busy="busy" :execute="execute" />
    <div v-if="displayOnly" class="panel standalone-activation">
      <div>
        <h2>仅切换显示器</h2>
        <p class="hint">
          无需局域网，无需配对；只切换画面，键鼠保持原样。请先验证两个方向都能切换。
        </p>
      </div>
      <button
        v-if="!state.running"
        class="button primary"
        :disabled="busy || !displayConfigured || state.preview"
        @click="$emit('action', { type: 'start' })"
      >
        启用显示器切换</button
      ><button
        v-else
        class="button secondary"
        :disabled="busy"
        @click="$emit('action', { type: 'stop' })"
      >
        停用显示器切换
      </button>
    </div>
    <WorkspaceOverview
      :state="state"
      :busy="busy"
      @switch="$emit('switch', $event)"
      @lock-peer="$emit('lockPeer')"
      @navigate="$emit('navigate', $event)"
    />
    <section v-if="!ready" class="setup-guide panel" aria-labelledby="setup-title">
      <div class="section-heading">
        <h2 id="setup-title">{{ state.running ? '连接准备' : '开始使用' }}</h2>
        <span>{{
          displayOnly ? '完成配置后，点击上方启用显示器切换' : '完成配置后，在设备页开始共享'
        }}</span>
      </div>
      <div class="setup-steps">
        <button
          v-for="(step, index) in steps"
          :key="step.page"
          class="setup-step"
          @click="$emit('navigate', step.page)"
        >
          <span :class="['step-number', { done: step.done }]"
            ><AppIcon v-if="step.done" name="tick" :size="16" /><template v-else>{{
              index + 1
            }}</template></span
          >
          <div>
            <strong>{{ step.title }}</strong>
            <p>{{ step.text }}</p>
            <small>{{
              step.done
                ? step.page === 'display'
                  ? '映射已保存 · 画面需实测'
                  : '已就绪'
                : '前往配置'
            }}</small>
          </div>
          <AppIcon name="chevron" :size="16" />
        </button>
      </div>
    </section>
    <div class="status-overview">
      <article class="status-card">
        <span class="status-label"
          ><AppIcon :name="displayOnly ? 'monitor' : 'link'" :size="18" />{{
            displayOnly ? '控制方式' : '设备连接'
          }}</span
        ><strong>{{
          displayOnly
            ? '本机直接发送 DDC'
            : state.connected
              ? state.peer?.name || '对端已连接'
              : state.running
                ? '等待对端连接'
                : '尚未连接'
        }}</strong>
        <p>
          {{
            displayOnly
              ? '不建立网络连接，不启动 Deskflow'
              : state.connected
                ? ready
                  ? '控制连接与键鼠引擎已就绪'
                  : '控制连接已认证，键鼠引擎未就绪'
                : '请保持两台电脑位于同一局域网'
          }}
        </p>
      </article>
      <article class="status-card">
        <span class="status-label"><AppIcon name="keyboard" :size="18" />键鼠当前归属</span
        ><strong>{{
          displayOnly ? '不转移键鼠' : state.switching ? '正在协调切换…' : activeName
        }}</strong>
        <p>
          {{
            displayOnly
              ? '键盘鼠标继续由当前系统或其他软件管理。'
              : '键鼠可独立跨屏，不代表显示器画面已切换。'
          }}
        </p>
      </article>
      <article class="status-card">
        <span class="status-label"><AppIcon name="monitor" :size="18" />画面指令状态</span
        ><strong>{{ picture }}</strong>
        <p>软件未确认物理画面，请以显示器实际显示为准。</p>
      </article>
    </div>
    <div class="shortcut-strip">
      <span
        ><AppIcon name="keyboard" :size="18" />{{
          displayOnly || state.config.role === 'host' ? '切换快捷键' : '快捷键由键鼠提供端设置'
        }}</span
      >
      <div v-if="displayOnly || state.config.role === 'host'" class="keys">
        <kbd v-for="key in shortcut" :key="key">{{ key }}</kbd>
      </div>
      <button class="text-button" @click="$emit('navigate', 'sharing')">
        {{ displayOnly ? '查看切换设置' : '查看共享设置' }}<AppIcon name="chevron" :size="14" />
      </button>
    </div>
    <p v-if="!displayOnly" class="hint">
      Mac 上 Alt 对应 Option；F 功能键可能需要同时按 Fn。异常时可尝试 Ctrl + Alt + Esc 恢复键鼠。
    </p>
  </section>
</template>

<style scoped>
.standalone-activation {
  display: flex;
  align-items: center;
  gap: 18px;
  justify-content: space-between;
  flex-wrap: wrap;
}
</style>
