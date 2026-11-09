<script setup lang="ts">
import { computed } from 'vue'
import { outputChoiceLabel, outputUnavailableReason } from '../shared/output-selection'
import type { Action, Config, Snapshot } from '../shared/types'
const props = defineProps<{ state: Snapshot; locked: boolean; dirty: boolean }>()
const method = defineModel<Config['displayMethod']>('method', { required: true })
const output = defineModel<string>('output', { required: true })
defineEmits<{ action: [action: Action] }>()
const outputs = computed(() => props.state.outputDisplays ?? [])
const selected = computed(() => outputs.value.find((screen) => screen.id === output.value))
const check = computed(() =>
  props.state.outputCheck?.id === output.value && !props.dirty ? props.state.outputCheck : null,
)
</script>

<template>
  <div class="panel">
    <div class="panel-heading">
      <div>
        <h2>切屏方式</h2>
        <p>支持 DDC 时优先使用 DDC。自动选源是可选的实验性兜底，两端需选择相同方式。</p>
      </div>
    </div>
    <label class="field">
      <span>如何切换共用显示器</span>
      <select v-model="method" :disabled="locked">
        <option value="ddc">DDC/CI 输入源切换（默认）</option>
        <option
          value="signal"
          :disabled="state.platform === 'linux' || state.config.mode !== 'sharing'"
        >
          断开输出，触发显示器自动选源（实验性）
        </option>
      </select>
    </label>
    <template v-if="method === 'signal'">
      <div class="inline-info" role="note">
        请先在显示器菜单开启自动选择输入源。两端需保持联网；目标输出恢复、键鼠归属确认和原输出停用成功后自动完成切换，无需手动确认。失败时会请求恢复输出。
        软件无法直接读取实际输入源，显示器实体按键改源后请用明确目标的按钮校正。
      </div>
      <label class="field">
        <span>允许断开的本机共用显示输出</span>
        <select v-model="output" :disabled="locked">
          <option value="" disabled>扫描并选择本机共用外接输出</option>
          <option v-if="output && !selected" :value="output">已保存的输出（当前未扫描到）</option>
          <option
            v-for="screen in outputs"
            :key="screen.id"
            :value="screen.id"
            :disabled="!!outputUnavailableReason(state, screen)"
          >
            {{ outputChoiceLabel(state, screen) }}
          </option>
        </select>
      </label>
      <p v-if="selected && outputUnavailableReason(state, selected)" class="validation-error">
        {{ outputUnavailableReason(state, selected) }}
      </p>
      <p v-if="!outputs.length" class="hint">
        点击上方“扫描本机输出”，选择共用屏幕后一次保存即可；扫描不会停用输出或修改已保存配置。
      </p>
      <p v-if="state.platform === 'darwin'" class="hint">
        Apple Silicon Mac 使用内置的原生输出控制，无需安装其他软件。 请保留 MacBook
        内置屏或另一块活动屏幕；暂不支持镜像输出。
      </p>
      <p v-else class="hint">
        多屏或镜像模式请使用 DDC。
        实体键鼠或其他软件输入可能提前唤醒输出，实际寻源与恢复速度取决于设备。
      </p>
      <p class="hint">
        画面仅由快捷键或按钮切换，鼠标边缘不会触发断开输出。停止共享、连接断开或主应用退出时会请求恢复本机输出。
      </p>
      <button
        class="button secondary compact"
        type="button"
        :disabled="locked || dirty || !output"
        @click="$emit('action', { type: 'checkOutput' })"
      >
        检测输出控制条件
      </button>
      <p v-if="dirty" class="hint">
        扫描可直接使用当前选择的切屏方式，无需先保存。选好输出后保存，再检测控制条件；扫描与检测都不会断开显示输出。
      </p>
      <p v-if="check" :class="check.available ? 'hint' : 'validation-error'" role="status">
        {{ check.detail }}
      </p>
    </template>
  </div>
</template>
