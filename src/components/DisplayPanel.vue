<script setup lang="ts">
import { computed, shallowRef } from 'vue'
import AppIcon from './AppIcon.vue'
import { useConfigDraft } from '../composables/useConfigDraft'
import type { Action, Snapshot } from '../shared/types'
const props = defineProps<{ state: Snapshot; busy: boolean }>()
const emit = defineEmits<{ action: [action: Action] }>()
const { draft, value } = useConfigDraft(() => props.state.config)
const confirmTest = shallowRef<number | null>(null)
const dirty = computed(() => JSON.stringify(value()) !== JSON.stringify(props.state.config))
const locked = computed(() => props.busy || props.state.running)
const valid = computed(
  () =>
    !!draft.displayId &&
    draft.localInput !== null &&
    draft.peerInput !== null &&
    draft.localInput !== draft.peerInput &&
    [draft.localInput, draft.peerInput].every((n) => Number.isInteger(n) && n >= 0 && n <= 65535),
)
const selected = computed(() => props.state.monitors.find((m) => m.id === draft.displayId))
function updateInput(which: 'localInput' | 'peerInput', event: Event) {
  const raw = (event.target as HTMLInputElement).value
  draft[which] = raw === '' ? null : Number(raw)
}
function testConfirmedInput() {
  if (confirmTest.value === null) return
  emit('action', { type: 'testInput', value: confirmTest.value })
  confirmTest.value = null
}
</script>
<template>
  <section>
    <div class="page-heading">
      <div>
        <span class="eyebrow">DISPLAY & INPUTS</span>
        <h1>同一块屏幕，各自的入口。</h1>
        <p>把每个输入源与电脑对应起来，切换时就知道该去哪里。</p>
      </div>
      <button class="button secondary" :disabled="locked" @click="emit('action', { type: 'scan' })">
        <AppIcon name="refresh" :size="16" />扫描显示器
      </button>
    </div>
    <form @submit.prevent="emit('action', { type: 'save', config: value() })">
      <div class="panel">
        <div class="panel-heading">
          <span class="section-icon"><AppIcon name="monitor" /></span>
          <div>
            <h2>选择共享显示器</h2>
            <p>两台电脑连接同一块物理显示器，各自选择它在本机的设备。</p>
          </div>
        </div>
        <div v-if="!state.monitors.length && !draft.displayId" class="empty-display">
          <span class="empty-icon"><AppIcon name="monitor" :size="38" /></span>
          <h3>还没有扫描显示器</h3>
          <p>连接外接显示器并开启其 DDC/CI，再点击“扫描显示器”。</p>
          <button
            type="button"
            class="button secondary"
            :disabled="locked"
            @click="emit('action', { type: 'scan' })"
          >
            扫描本机显示器
          </button>
        </div>
        <template v-else
          ><label class="field"
            ><span>本机的外接显示器</span
            ><select v-model="draft.displayId" :disabled="locked">
              <option value="" disabled>请选择显示器</option>
              <option v-if="draft.displayId && !selected" :value="draft.displayId">
                已保存的设备（扫描后确认连接）
              </option>
              <option v-for="monitor in state.monitors" :key="monitor.id" :value="monitor.id">
                {{ monitor.name }} · {{ monitor.id }}
              </option>
            </select></label
          >
          <div v-if="selected" class="display-info">
            <AppIcon name="cable" :size="18" /><span>{{
              selected.readable
                ? `上次扫描读取到的输入源：${selected.currentInput}`
                : '暂时无法读取当前输入值；是否支持写入需单独测试。'
            }}</span>
          </div></template
        >
      </div>
      <div class="panel">
        <div class="panel-heading">
          <span class="section-icon"><AppIcon name="switch" /></span>
          <div>
            <h2>电脑与输入源映射</h2>
            <p>填写显示器实际使用的数值，接口名称和数值可能因型号而异。</p>
          </div>
        </div>
        <div class="mapping-grid">
          <div class="mapping-card">
            <span class="mapping-label"
              ><AppIcon name="laptop" :size="18" />本机 · {{ state.config.deviceName }}</span
            ><label class="field"
              ><span>本机输入源数值</span
              ><input
                :value="draft.localInput ?? ''"
                type="number"
                min="0"
                max="65535"
                step="1"
                placeholder="填写已验证的数值"
                :disabled="locked"
                @input="updateInput('localInput', $event)" /></label
            ><small>显示本机画面时对应的输入源。</small>
          </div>
          <span class="mapping-arrow"><AppIcon name="switch" /></span>
          <div class="mapping-card">
            <span class="mapping-label"><AppIcon name="laptop" :size="18" />另一台电脑</span
            ><label class="field"
              ><span>另一台输入源数值</span
              ><input
                :value="draft.peerInput ?? ''"
                type="number"
                min="0"
                max="65535"
                step="1"
                placeholder="填写已验证的数值"
                :disabled="locked"
                @input="updateInput('peerInput', $event)" /></label
            ><small>切到另一台电脑时发送的输入源。</small>
          </div>
        </div>
        <div class="reference-values">
          <span>常见参考值</span><code>VGA 1</code><code>DVI 3</code><code>HDMI 17</code
          ><code>DP 15</code><small>请以你的显示器实测为准。</small>
        </div>
        <p class="hint">对端要反向填写：例如本机 3 / 对端 1，则另一台填写本机 1 / 对端 3。</p>
        <div class="form-footer">
          <span>{{
            state.running
              ? '共享运行中，停止后可修改。'
              : dirty
                ? '有未保存的修改'
                : '映射设置保存在本机'
          }}</span
          ><button type="submit" class="button primary" :disabled="locked || !dirty || !valid">
            保存输入源映射
          </button>
        </div>
      </div>
    </form>
    <div class="panel test-panel">
      <div>
        <h2>单独测试显示器切换</h2>
        <p>测试会立即改变物理画面，请确保目标电脑已开机并连接。</p>
      </div>
      <div class="button-group">
        <button
          class="button secondary"
          :disabled="locked || dirty || !valid"
          @click="confirmTest = state.config.localInput"
        >
          测试本机输入</button
        ><button
          class="button secondary"
          :disabled="locked || dirty || !valid"
          @click="confirmTest = state.config.peerInput"
        >
          测试另一台输入
        </button>
      </div>
      <div v-if="confirmTest !== null" class="test-confirm">
        <span
          >即将发送输入源 <b>{{ confirmTest }}</b
          >。若无画面，可用显示器实体按键恢复。</span
        ><button class="button primary compact" :disabled="busy" @click="testConfirmedInput">
          立即测试</button
        ><button class="text-button" @click="confirmTest = null">取消</button>
      </div>
    </div>
  </section>
</template>
