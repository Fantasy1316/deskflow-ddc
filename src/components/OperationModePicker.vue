<script setup lang="ts">
import type { Snapshot, OperationMode } from '../shared/types'
import type { ActionRunner } from '../shared/workspace'
import AppIcon from './AppIcon.vue'
defineProps<{ state: Snapshot; busy: boolean; execute: ActionRunner }>()
const modes: { id: OperationMode; title: string; description: string; icon: 'link' | 'monitor' }[] =
  [
    {
      id: 'sharing',
      title: '完整共享',
      description: '局域网连接，同步键鼠、剪贴板与文件',
      icon: 'link',
    },
    {
      id: 'display-only',
      title: '仅切换显示器',
      description: '无需局域网或配对，仅发送输入源指令',
      icon: 'monitor',
    },
  ]
</script>
<template>
  <section class="mode-picker" aria-label="运行模式">
    <div class="mode-options">
      <button
        v-for="mode in modes"
        :key="mode.id"
        :class="['mode-option', { selected: state.config.mode === mode.id }]"
        :aria-pressed="state.config.mode === mode.id"
        :disabled="busy || state.running"
        @click="
          state.config.mode !== mode.id &&
          execute({ type: 'save', config: { ...state.config, mode: mode.id } })
        "
      >
        <AppIcon :name="mode.icon" :size="21" /><span
          ><strong>{{ mode.title }}</strong
          ><small>{{ mode.description }}</small></span
        ><AppIcon v-if="state.config.mode === mode.id" name="tick" :size="18" />
      </button>
    </div>
    <p v-if="state.running" class="hint">
      {{
        state.config.mode === 'display-only'
          ? '停用显示器切换后可更换模式。'
          : '停止共享后可更换模式。'
      }}
    </p>
  </section>
</template>
<style scoped>
.mode-picker {
  margin-bottom: 20px;
}
.mode-options {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}
.mode-option {
  display: flex;
  gap: 12px;
  align-items: center;
  text-align: left;
  padding: 18px;
  border: 1px solid #dfe6df;
  border-radius: 10px;
  background: #fff;
  min-width: 0;
}
.mode-option.selected {
  background: #eef4eb;
  border-color: #89a284;
}
.mode-option > svg {
  flex-shrink: 0;
  color: #557850;
}
.mode-option > svg:last-child {
  margin-left: auto;
}
.mode-option strong {
  font-size: 14px;
}
.mode-option small {
  display: block;
  font-size: 12px;
  color: #63736b;
  line-height: 1.7;
  margin-top: 5px;
}
.mode-option:disabled {
  opacity: 0.7;
}
@media (max-width: 760px) {
  .mode-options {
    grid-template-columns: 1fr;
  }
}
</style>
