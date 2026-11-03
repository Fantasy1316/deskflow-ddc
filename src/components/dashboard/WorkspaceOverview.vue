<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from '../AppIcon.vue'
import type { Snapshot } from '../../shared/types'
import { deviceName, displayMappingValid, pictureCommandLabel } from '../../shared/workspace'
const props = defineProps<{ state: Snapshot; busy: boolean }>()
defineEmits<{ switch: [target: 'local' | 'peer']; lockPeer: []; navigate: [page: string] }>()
const displayOnly = computed(() => props.state.config.mode === 'display-only')
const ready = computed(() =>
  displayOnly.value
    ? props.state.running && displayMappingValid(props.state.config)
    : props.state.connected && props.state.engine === 'ready',
)
const disabled = computed(() => !ready.value || props.busy || props.state.switching)
const activeName = computed(() =>
  props.state.activeDeviceId
    ? deviceName(props.state, props.state.activeDeviceId)
    : '尚未建立键鼠共享',
)
const controlLabel = computed(() =>
  displayOnly.value
    ? props.state.switching
      ? '正在发送输入源指令…'
      : '仅切换画面 · 键鼠不转移'
    : props.state.switching
      ? '正在协调切换…'
      : '键鼠：' + activeName.value,
)
const localActionLabel = computed(() =>
  displayOnly.value ? '切到本机输入' : '切到 ' + props.state.config.deviceName,
)
const peerActionLabel = computed(() =>
  displayOnly.value ? '切到另一输入' : '切到 ' + (props.state.peer?.name || '另一台电脑'),
)
const monitorName = computed(
  () =>
    props.state.monitors.find((m) => m.id === props.state.config.displayId)?.name ||
    (props.state.config.displayId ? '已配置的显示器' : '等待选择显示器'),
)
const picture = computed(() => pictureCommandLabel(props.state))
const videoMode = computed(() =>
  props.state.config.role === 'host'
    ? props.state.config.crossScreenVideo
    : (props.state.sessionLayout?.video ?? 'manual'),
)
const connectionLabel = computed(() =>
  displayOnly.value
    ? props.state.running
      ? '显示器切换已启用'
      : '等待启用显示器切换'
    : props.state.engine === 'error'
      ? '引擎连接失败'
      : ready.value
        ? '共享运行中'
        : props.state.connected
          ? '键鼠引擎连接中'
          : props.state.running
            ? '等待建立连接'
            : '尚未开始共享',
)
</script>
<template>
  <section class="workspace-card" aria-labelledby="workspace-title">
    <div class="workspace-caption">
      <h2 id="workspace-title" class="subtle-label">
        <i class="tiny-dot" :class="{ on: ready }"></i>共享工作空间
      </h2>
      <span class="light-badge">{{ connectionLabel }}</span>
    </div>
    <div class="workspace-visual">
      <div class="computer-node">
        <div class="computer-icon local">
          <AppIcon name="laptop" :size="34" /><span class="node-dot" :class="{ on: ready }"></span>
        </div>
        <strong :title="state.config.deviceName">{{ state.config.deviceName }}</strong
        ><small
          >本机 ·
          {{
            displayOnly ? '仅控制输入源' : state.config.role === 'host' ? '提供键鼠' : '接收键鼠'
          }}</small
        ><span class="input-badge">输入源 {{ state.config.localInput ?? '待配置' }}</span>
      </div>
      <div class="connection-line" aria-hidden="true">
        <span></span><AppIcon name="switch" :size="18" /><span></span>
      </div>
      <div class="shared-display">
        <div class="display-bezel">
          <div class="display-surface">
            <span class="surface-orbit orbit-one" aria-hidden="true"></span
            ><span class="surface-orbit orbit-two" aria-hidden="true"></span
            ><span class="surface-orbit orbit-three" aria-hidden="true"></span>
            <div class="surface-label">
              <AppIcon name="monitor" :size="26" /><strong>{{ monitorName }}</strong
              ><small>{{ picture }}</small>
            </div>
          </div>
          <i></i>
        </div>
        <div class="display-stand"></div>
        <div class="display-foot"></div>
        <span class="display-caption">同一块显示器 · 画面以实际显示为准</span>
      </div>
      <div class="connection-line" aria-hidden="true">
        <span></span><AppIcon name="switch" :size="18" /><span></span>
      </div>
      <div class="computer-node">
        <div class="computer-icon">
          <AppIcon name="laptop" :size="34" /><span
            class="node-dot"
            :class="{ on: state.connected }"
          ></span>
        </div>
        <strong :title="displayOnly ? '另一输入设备' : state.peer?.name || '另一台电脑'">{{
          displayOnly ? '另一输入设备' : state.peer?.name || '另一台电脑'
        }}</strong
        ><small
          >{{ displayOnly ? '无需运行对端应用' : state.connected ? '已连接' : '等待连接'
          }}<template v-if="!displayOnly">
            · {{ state.config.role === 'host' ? '接收键鼠' : '提供键鼠' }}</template
          ></small
        ><span class="input-badge">输入源 {{ state.config.peerInput ?? '待配置' }}</span>
      </div>
    </div>
    <div class="workspace-controls">
      <div class="active-label">
        <span class="pulse-dot" :class="{ on: ready }"></span
        ><span :title="controlLabel">{{ controlLabel }}</span>
      </div>
      <div class="button-group">
        <button
          v-if="
            !displayOnly &&
            state.config.role === 'host' &&
            state.peer?.platform === 'win32' &&
            state.peer.lockControl
          "
          class="button secondary compact"
          :title="'锁定 ' + state.peer.name"
          :disabled="!ready || busy"
          @click="$emit('lockPeer')"
        >
          <span class="button-label">锁定 {{ state.peer.name }}</span></button
        ><button
          class="button secondary compact"
          :title="localActionLabel"
          :disabled="disabled"
          @click="$emit('switch', 'local')"
        >
          <span class="button-label">{{ localActionLabel }}</span></button
        ><button
          class="button primary compact"
          :title="peerActionLabel"
          :disabled="disabled"
          @click="$emit('switch', 'peer')"
        >
          <AppIcon name="switch" :size="16" /><span class="button-label">{{
            peerActionLabel
          }}</span>
        </button>
      </div>
    </div>
    <div class="workspace-note">
      <span>{{
        displayOnly
          ? '所有指令从本机发送，若切走后无法切回，请使用显示器实体按键恢复。'
          : !ready
            ? '控制连接与键鼠引擎就绪后可切换，请先完成配对并开始共享。'
            : videoMode === 'manual'
              ? '边缘只转移键鼠，画面由快捷键或按钮切换'
              : videoMode === 'keep'
                ? '按独立屏幕配置转移键鼠'
                : '已启用鼠标跨屏时跟随切换画面'
      }}</span
      ><button class="text-button" @click="$emit('navigate', 'display')">
        检查显示器配置<AppIcon name="chevron" :size="14" />
      </button>
    </div>
  </section>
</template>
<style scoped>
.light-badge {
  font-size: 9px;
  background: #f1f4ed;
  border: 1px solid #e4eadc;
  color: #839076;
  padding: 5px 9px;
  border-radius: 5px;
}
.workspace-card {
  background: linear-gradient(125deg, #fff 50%, #f7f9f5);
  border: 1px solid #e0e7df;
  border-radius: 13px;
  box-shadow: 0 4px 20px #40563b03;
  overflow: hidden;
}
.workspace-caption {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 20px 23px;
}
.subtle-label {
  display: flex;
  gap: 8px;
  align-items: center;
  font-size: 11px;
  color: #6e8174;
}
.tiny-dot {
  width: 5px;
  height: 5px;
  background: #88a578;
}
.workspace-visual {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 16px;
  padding: 12px 26px 27px;
  min-height: 235px;
}
.computer-node {
  display: flex;
  align-items: center;
  flex-direction: column;
  min-width: 118px;
  max-width: 130px;
}
.computer-icon {
  width: 64px;
  height: 64px;
  border: 1px solid #e4e9df;
  border-radius: 17px;
  background: #f6f8f4;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #93a18e;
  position: relative;
}
.computer-icon.local {
  background: #ecf2e4;
  border-color: #d9e4ce;
  color: #6f8561;
}
.node-dot {
  position: absolute;
  right: 3px;
  bottom: 4px;
  width: 9px;
  height: 9px;
  border: 2px solid #fff;
  background: #ccd4c8;
  border-radius: 50%;
}
.computer-node strong {
  margin-top: 14px;
  font-size: 11px;
  max-width: 125px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}
.computer-node small {
  margin-top: 6px;
  font-size: 9px;
  color: #98a193;
}
.input-badge {
  font-size: 9px;
  margin-top: 12px;
  color: #91a185;
  border: 1px solid #e2e9da;
  background: #f6f8f0;
  border-radius: 5px;
  padding: 4px 7px;
}
.connection-line {
  display: flex;
  align-items: center;
  gap: 6px;
  color: #bdcaba;
  flex: 1;
  max-width: 86px;
  min-width: 20px;
  margin-top: -39px;
}
.connection-line span {
  flex: 1;
  border-bottom: 1px dashed #ccd6c4;
}
.shared-display {
  width: 241px;
  min-width: 200px;
  display: flex;
  align-items: center;
  flex-direction: column;
  position: relative;
}
.display-bezel {
  background: #425247;
  border-radius: 9px;
  padding: 6px 6px 12px;
  width: 100%;
  box-shadow: 0 8px 20px #3449360b;
  position: relative;
}
.display-bezel > i {
  position: absolute;
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background: #99ab96;
  bottom: 5px;
  left: 50%;
}
.display-surface {
  height: 130px;
  background: #e9efde;
  overflow: hidden;
  position: relative;
  border-radius: 4px;
}
.surface-orbit {
  position: absolute;
  display: block;
  border: 1px solid #d6e0c2;
  border-radius: 50%;
  width: 260px;
  height: 260px;
  left: 62px;
  top: -75px;
}
.orbit-two {
  width: 200px;
  height: 200px;
  left: 90px;
  top: -45px;
}
.orbit-three {
  width: 145px;
  height: 145px;
  left: 118px;
  top: -18px;
  background: #dce6ce66;
}
.surface-label {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: #66805c;
  gap: 9px;
}
.surface-label strong {
  font-size: 12px;
  font-weight: 550;
}
.surface-label small {
  font-size: 8px;
  letter-spacing: 1.2px;
  color: #9aaa8c;
}
.display-stand {
  height: 22px;
  width: 33px;
  background: linear-gradient(#72816d, #899880);
  clip-path: polygon(15% 0, 85% 0, 100% 100%, 0 100%);
}
.display-foot {
  height: 4px;
  width: 90px;
  border-radius: 6px 6px 2px 2px;
  background: #7f8e78;
}
.display-caption {
  margin-top: 10px;
  font-size: 9px;
  color: #a0aa96;
  letter-spacing: 0.5px;
}
.workspace-controls {
  border-top: 1px solid #e9eee4;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 22px;
  background: #fbfcf9;
}
.active-label {
  display: flex;
  align-items: center;
  gap: 8px;
  color: #8b9b87;
  font-size: 10px;
}
.pulse-dot {
  width: 5px;
  height: 5px;
}
.workspace-card {
  margin-bottom: 20px;
}
.workspace-caption {
  padding: 20px 24px;
}
.subtle-label {
  font-size: 14px;
  font-weight: 600;
  letter-spacing: 0;
}
.light-badge,
.input-badge,
.computer-node small,
.display-caption {
  font-size: 12px;
  color: #62745e;
}
.computer-node {
  min-width: 0;
  width: 150px;
  max-width: 150px;
  flex-shrink: 0;
}
.computer-node strong {
  font-size: 13px;
  max-width: 100%;
}
.shared-display {
  width: 270px;
  min-width: 220px;
}
.display-surface {
  height: 155px;
}
.surface-label {
  padding: 16px;
  gap: 10px;
}
.surface-label strong {
  font-size: 14px;
  text-align: center;
  overflow-wrap: anywhere;
}
.surface-label small {
  font-size: 12px;
  letter-spacing: 0;
  color: #65795c;
  text-align: center;
  overflow-wrap: anywhere;
  line-height: 1.6;
}
.display-caption {
  margin-top: 12px;
  text-align: center;
  line-height: 1.6;
}
.workspace-controls {
  gap: 16px;
  flex-wrap: nowrap;
  padding: 18px 24px;
}
.active-label {
  color: #62745e;
  font-size: 12px;
  min-width: 0;
  flex: 1;
}
.active-label > span:last-child {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.button-group {
  min-width: 0;
  max-width: 72%;
  flex-wrap: nowrap;
  margin-left: auto;
}
.button-group .button {
  min-width: 0;
  max-width: 200px;
  height: 36px;
  padding: 0 14px;
  white-space: nowrap;
  text-align: left;
}
.button-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.button-group .button > svg,
.pulse-dot {
  flex-shrink: 0;
}
.workspace-note {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 24px;
  border-top: 1px solid #e9eee4;
  font-size: 12px;
  color: #63736b;
  line-height: 1.7;
  flex-wrap: wrap;
}
.workspace-note .text-button {
  font-size: 12px;
}
@media (min-width: 1350px) {
  .workspace-visual {
    padding-top: 22px;
    padding-bottom: 32px;
  }
  .shared-display {
    width: 300px;
  }
  .display-surface {
    height: 170px;
  }
}
@media (max-width: 1050px) {
  .workspace-visual {
    gap: 10px;
    padding: 12px 20px 24px;
  }
  .computer-node {
    width: 125px;
  }
  .shared-display {
    width: 230px;
    min-width: 210px;
  }
  .connection-line {
    min-width: 10px;
  }
}
@media (max-width: 760px) {
  .workspace-visual {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 20px;
  }
  .shared-display {
    order: -1;
    grid-column: 1 / -1;
    width: min(100%, 270px);
    min-width: 0;
    max-width: none;
    margin: 0 auto;
  }
  .connection-line {
    display: none;
  }
  .computer-node {
    width: 100%;
    max-width: none;
  }
  .workspace-controls,
  .workspace-caption,
  .workspace-note {
    padding-left: 18px;
    padding-right: 18px;
  }
  .button-group {
    margin-left: 0;
    max-width: 100%;
    width: 100%;
  }
  .workspace-controls {
    flex-wrap: wrap;
  }
  .active-label {
    flex-basis: 100%;
  }
  .button-group .button {
    flex: 1;
    max-width: none;
  }
}
</style>
