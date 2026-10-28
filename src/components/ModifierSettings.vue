<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import type { ModifierMapping } from '../shared/types'

const props = defineProps<{
  locked: boolean
  isHost: boolean
  connected: boolean
  hostMapping?: ModifierMapping
}>()
const mapping = defineModel<ModifierMapping>({ required: true })
const displayedMapping = computed({
  get: () => (props.isHost ? mapping.value : (props.hostMapping ?? 'original')),
  set: (value) => {
    if (props.isHost) mapping.value = value
  },
})
</script>

<template>
  <div class="panel">
    <div class="panel-heading">
      <span class="section-icon"><AppIcon name="keyboard" /></span>
      <div>
        <h2>跨系统修饰键映射</h2>
        <p>统一 Mac 与 Windows 的常用编辑快捷键，仅作用于发往接收端的键盘输入。</p>
      </div>
    </div>
    <div v-if="!isHost" class="inline-info">
      此设置由键鼠提供端管理。{{
        connected ? '下方显示提供端的设置。' : '连接后自动显示提供端的设置。'
      }}
    </div>
    <label class="field">
      <span>修饰键映射模式</span>
      <select v-model="displayedMapping" :disabled="locked || !isHost">
        <option value="original">保持原始按键（默认）</option>
        <option value="common">统一常用快捷键（Command ↔ Ctrl）</option>
      </select>
    </label>
    <div v-if="displayedMapping === 'common'" class="inline-info">
      Mac 提供键鼠：Command → Ctrl，Control → Win。Windows 提供键鼠：Ctrl → Command，Win → Control。
      例如在 Mac 键盘上按 Command + C / V，可在 Windows 中复制 / 粘贴。
    </div>
    <p v-else>跨系统时保留 Command ↔ Win、Control ↔ Ctrl 的原始对应关系。</p>
    <p>
      同系统共享及本机操作保持原样。开启后会交换所有相关组合键；应用切换等组合不同的快捷键仍按目标系统规则使用。
      请先停止共享，修改并保存后重新开始共享。
    </p>
  </div>
</template>
