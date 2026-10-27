<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import ScreenLayoutEditor from './ScreenLayoutEditor.vue'
import type { Config, Snapshot } from '../shared/types'
const props = defineProps<{ locked: boolean; isHost: boolean; state: Snapshot }>()
const side = defineModel<Config['layoutSide']>('side', { required: true })
const video = defineModel<Config['crossScreenVideo']>('video', { required: true })
const retained = defineModel<string>('retained', { required: true })
const displayedSide = computed({
  get: () => (props.isHost ? side.value : (props.state.sessionLayout?.hostSide ?? 'off')),
  set: (value) => {
    if (props.isHost) side.value = value
  },
})
const displayedVideo = computed({
  get: () => (props.isHost ? video.value : (props.state.sessionLayout?.video ?? 'manual')),
  set: (value) => {
    if (props.isHost) video.value = value
  },
})
const hostName = computed(() =>
  props.isHost ? props.state.config.deviceName : (props.state.peer?.name ?? '键鼠提供端'),
)
const peerName = computed(() =>
  props.isHost ? (props.state.peer?.name ?? '另一台电脑') : props.state.config.deviceName,
)
const retainedAvailable = computed(
  () =>
    retained.value === '' ||
    props.state.cursorDisplays.some((d) =>
      retained.value === 'internal' ? d.internal : d.id === retained.value,
    ),
)
</script>

<template>
  <div class="panel">
    <div class="panel-heading">
      <span class="section-icon"><AppIcon name="monitor" /></span>
      <div>
        <h2>屏幕布局与鼠标跨屏</h2>
        <p>按电脑的相对位置设置跨屏方向，支持相同或不同操作系统。</p>
      </div>
    </div>
    <div v-if="!isHost" class="inline-info">
      布局和跨屏画面行为由键鼠提供端管理。{{
        state.connected ? '下方显示本次连接的布局。' : '连接后自动显示提供端的布局。'
      }}本机独立屏幕可在停止共享后设置。
    </div>
    <ScreenLayoutEditor
      v-model="displayedSide"
      :disabled="locked || !isHost"
      :read-only="!isHost"
      :host-name="hostName"
      :peer-name="peerName"
    />
    <label class="field"
      ><span>键鼠提供端相对接收端的位置</span
      ><select v-model="displayedSide" :disabled="locked || !isHost">
        <option value="off">关闭鼠标边缘跨屏，仅使用按钮和快捷键</option>
        <option value="left">左侧</option>
        <option value="right">右侧</option>
        <option value="top">上方</option>
        <option value="bottom">下方</option>
      </select></label
    >
    <label class="field"
      ><span>鼠标跨屏时，共用显示器如何切换</span
      ><select v-model="displayedVideo" :disabled="locked || !isHost">
        <option value="manual">画面仅手动切换，边缘只转移键鼠（推荐）</option>
        <option value="follow">允许边缘跟随键鼠切换画面</option>
        <option value="keep">目标有独立屏幕时仅转移键鼠，否则允许边缘切换画面</option>
      </select></label
    >
    <label class="field"
      ><span>本机独立屏幕（共用显示器显示另一台电脑时仍可见）</span
      ><select v-model="retained" :disabled="locked">
        <option value="">没有独立屏幕，仅使用共用显示器</option>
        <option v-if="retained === 'internal'" value="internal">原配置的内置屏幕</option>
        <option v-if="!retainedAvailable" :value="retained">所选屏幕已断开，请重新选择</option>
        <option v-for="display in state.cursorDisplays" :key="display.id" :value="display.id">
          {{ display.name }} · {{ display.bounds.width }} × {{ display.bounds.height }}
        </option>
      </select></label
    >
    <p>
      每台电脑分别选择自己的独立屏幕。只有共用显示器时请选择“没有独立屏幕”；请勿将共用显示器选作独立屏幕。
    </p>
    <div v-if="displayedSide !== 'off'" class="inline-info">
      <template v-if="displayedVideo === 'manual'">
        边缘不会切换显示器输入源。目标有独立屏幕或正在显示共用屏幕时，才允许转移键鼠；否则请使用按钮或快捷键切换画面。
      </template>
      <template v-else> 当前允许边缘触发画面切换，碰到指定边缘可能切换显示器输入源。 </template>
      保存并重新开始共享后生效。按钮和切换快捷键始终用于切换共用屏幕画面。
    </div>
  </div>
</template>
