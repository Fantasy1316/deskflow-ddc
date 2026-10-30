<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import InputSourceField from './InputSourceField.vue'
import DisplayTestStep from './DisplayTestStep.vue'
import SettingsLock from './SettingsLock.vue'
import { useConfigDraft } from '../composables/useConfigDraft'
import { displayMappingValid, type ActionRunner } from '../shared/workspace'
import type { Action, Snapshot } from '../shared/types'
const props = defineProps<{ state: Snapshot; busy: boolean; execute: ActionRunner }>()
const emit = defineEmits<{ action: [action: Action] }>()
const { draft, value } = useConfigDraft(() => props.state.config)
const displayOnly = computed(() => props.state.config.mode === 'display-only')
const dirty = computed(() => JSON.stringify(value()) !== JSON.stringify(props.state.config))
const locked = computed(() => props.busy || props.state.running)
const valid = computed(() => displayMappingValid(value()))
const selected = computed(() => props.state.monitors.find((m) => m.id === draft.displayId))
const peerName = computed(() =>
  displayOnly.value ? '另一输入设备' : props.state.peer?.name || '另一台电脑',
)
const testReason = computed(() =>
  props.state.running
    ? displayOnly.value
      ? '请先停用显示器切换再测试。'
      : '共享运行中，请先停止共享再测试。'
    : dirty.value
      ? '请先保存输入源映射，再测试。'
      : !valid.value
        ? '请先选择显示器，为两台电脑填写不同的有效数值。'
        : '正在处理操作，请稍候。',
)
</script>
<template>
  <section>
    <div class="page-heading">
      <div>
        <h1>显示器与输入</h1>
        <p>先确认显示器，再设置输入源，最后验证两个方向的实际画面。</p>
      </div>
      <button class="button secondary" :disabled="locked" @click="emit('action', { type: 'scan' })">
        <AppIcon name="refresh" :size="16" />扫描显示器
      </button>
    </div>
    <SettingsLock
      v-if="state.running"
      :display-only="displayOnly"
      :busy="busy"
      @stop="emit('action', { type: 'stop' })"
    />
    <form @submit.prevent="emit('action', { type: 'save', config: value() })">
      <div class="panel">
        <div class="panel-heading">
          <span class="step-number">1</span>
          <div>
            <h2>选择共享显示器</h2>
            <p>能识别显示器不等于支持 DDC 切换；读取与写入能力需要分别判断。</p>
          </div>
        </div>
        <div v-if="!state.monitors.length" class="inline-info">
          <strong>未发现可控制的显示器</strong>
          <p>
            请确认外接屏已连接并开机，在显示器菜单开启 DDC/CI；转接坞、线缆和连接方式也会影响检测。
          </p>
        </div>
        <label class="field"
          ><span>本机连接的共享显示器</span
          ><select v-model="draft.displayId" :disabled="locked">
            <option value="" disabled>请选择显示器</option>
            <option v-if="draft.displayId && !selected" :value="draft.displayId">
              已保存的显示器（当前未扫描到）
            </option>
            <option
              v-for="(monitor, index) in state.monitors"
              :key="monitor.id"
              :value="monitor.id"
            >
              {{ monitor.name
              }}{{
                state.monitors.filter((m) => m.name === monitor.name).length > 1
                  ? ` · 设备 ${index + 1}`
                  : ''
              }}
            </option>
          </select></label
        >
        <div v-if="selected" class="display-capabilities">
          <span class="badge success">已发现显示器</span
          ><span class="badge neutral">{{
            selected.readable && selected.currentInput !== undefined
              ? `上次扫描读到输入值 ${selected.currentInput}`
              : '当前输入值未读到'
          }}</span
          ><span class="badge neutral">切换能力需实测</span>
        </div>
        <p v-if="selected" class="hint">
          {{
            selected.readable && selected.currentInput !== undefined
              ? '该数值对应上次扫描时的当前输入，不是每个接口的编号。切到另一台电脑的画面后再读取，才能比较两端数值。'
              : '读不到不代表不支持写入。可以参考接口数值并进行下方测试，也可从显示器说明书或其他 DDC 工具获取数值。'
          }}
        </p>
        <details v-if="draft.displayId" class="monitor-details">
          <summary>显示器设备标识</summary>
          <code class="copyable-text">{{ draft.displayId }}</code>
          <p>此标识用于选择设备，不是输入源数值。</p>
        </details>
      </div>
      <div class="panel">
        <div class="panel-heading">
          <span class="step-number">2</span>
          <div>
            <h2>把输入源对应到电脑</h2>
            <p>根据线缆所接接口选择参考值。不确定时使用自定义数值，以实际测试为准。</p>
          </div>
        </div>
        <div class="mapping-grid">
          <div class="mapping-card">
            <span class="mapping-label"
              ><AppIcon name="laptop" :size="18" />{{ state.config.deviceName }} · 本机</span
            ><InputSourceField v-model="draft.localInput" :disabled="locked" name="本机" />
          </div>
          <span class="mapping-arrow"><AppIcon name="switch" /></span>
          <div class="mapping-card">
            <span class="mapping-label"
              ><AppIcon name="laptop" :size="18" />{{ peerName }} · 对端</span
            ><InputSourceField v-model="draft.peerInput" :disabled="locked" name="另一台电脑" />
          </div>
        </div>
        <p class="hint">
          USB-C 常承载 DisplayPort 信号，但对应数值由显示器决定。HDMI / DP
          预设仅供参考，不能据此认定已支持。
        </p>
        <p
          v-if="draft.localInput !== null && draft.localInput === draft.peerInput"
          class="validation-error"
        >
          两台电脑需要不同的输入源数值，请检查接口映射。
        </p>
        <div class="mapping-example">
          <strong>{{ displayOnly ? '两个输入源的对应关系' : '另一端需要反向填写' }}</strong>
          <p v-if="!displayOnly">
            {{ state.config.deviceName }}：本机 {{ draft.localInput ?? 'A' }} / 对端
            {{ draft.peerInput ?? 'B' }} → {{ peerName }}：本机 {{ draft.peerInput ?? 'B' }} / 对端
            {{ draft.localInput ?? 'A' }}
          </p>
          <p v-if="displayOnly">
            本机输入 {{ draft.localInput ?? '待设置' }} / 另一输入
            {{ draft.peerInput ?? '待设置' }}。所有指令由本机发送，另一端无需配对。
          </p>
        </div>
        <div class="form-footer">
          <span>{{
            state.running
              ? displayOnly
                ? '停用切换后可修改'
                : '停止共享后可修改'
              : dirty
                ? '有未保存的修改'
                : valid
                  ? '映射已保存，实际切换效果仍需测试'
                  : '请选择显示器并填写两个不同的输入值'
          }}</span
          ><button type="submit" class="button primary" :disabled="locked || !dirty || !valid">
            保存输入源映射
          </button>
        </div>
      </div>
    </form>
    <DisplayTestStep
      :display-only="displayOnly"
      :config="value()"
      :local-name="state.config.deviceName"
      :peer-name="peerName"
      :disabled="locked || dirty || !valid"
      :reason="testReason"
      :execute="execute"
    />
    <details class="panel ddc-help">
      <summary>如何判断显示器是否支持 DDC 输入源切换？</summary>
      <p>1. 查看显示器说明书或菜单，确认 DDC/CI 已开启。</p>
      <p>2. 选择显示器；能读取输入值只说明读取成功，读不到也不能直接判断不支持。</p>
      <p>
        3.
        {{
          displayOnly ? '停用显示器切换' : '停止共享'
        }}，分别测试两台电脑的输入值，并亲眼确认画面。两个方向都能切换，才适合用于日常使用。
      </p>
      <p>
        如果只能单向切换，请检查{{
          displayOnly ? '本机发送指令的能力、输入映射、DDC' : '另一端的映射、DDC'
        }}
        支持和连接线。显示器可能只接受当前活动输入端发送的指令。
      </p>
    </details>
  </section>
</template>
