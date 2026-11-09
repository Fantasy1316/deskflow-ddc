<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import type { Action, Snapshot } from '../../shared/types'

const props = defineProps<{ state: Snapshot; busy: boolean }>()
defineEmits<{ action: [action: Action] }>()
const expanded = shallowRef(false)
const checking = computed(() => props.state.displaySource?.kind === 'checking')
const unknown = computed(
  () =>
    props.state.displaySource?.kind === 'unknown' ||
    props.state.displayStatus !== 'commanded' ||
    !props.state.displayDeviceId,
)
const visible = computed(
  () => props.state.config.mode === 'sharing' && props.state.connected && !!props.state.peer,
)
const disabled = computed(
  () => props.busy || props.state.switching || props.state.emergencyStopping || checking.value,
)
const reason = computed(() =>
  props.state.displaySource?.kind === 'unknown' ? props.state.displaySource.reason : '',
)
watch(
  () => {
    const source = props.state.displaySource
    return source?.kind === 'ddc' || source?.kind === 'manual'
      ? `${source.kind}:${source.device}`
      : null
  },
  (confirmed) => {
    if (confirmed) expanded.value = false
  },
)
</script>

<template>
  <div v-if="visible" class="display-source-picker" :class="{ pending: unknown || checking }">
    <template v-if="checking">
      <span role="status">正在读取共用显示器当前输入源…</span>
    </template>
    <template v-else-if="unknown || expanded">
      <div class="source-explanation">
        <strong>共用显示器现在显示哪台电脑？</strong>
        <p v-if="reason">{{ reason }}</p>
        <p>按实际画面选择；仅更新来源记录，不切屏、不转移键鼠。</p>
      </div>
      <div class="source-actions">
        <button
          class="button secondary compact"
          :title="'当前显示本机：' + state.config.deviceName"
          :disabled="disabled"
          @click="$emit('action', { type: 'confirmDisplaySource', target: 'local' })"
        >
          <span class="button-label">本机：{{ state.config.deviceName }}</span>
        </button>
        <button
          class="button secondary compact"
          :title="'当前显示另一台：' + state.peer!.name"
          :disabled="disabled"
          @click="$emit('action', { type: 'confirmDisplaySource', target: 'peer' })"
        >
          <span class="button-label">另一台：{{ state.peer!.name }}</span>
        </button>
        <button
          v-if="state.config.displayMethod === 'ddc'"
          class="text-button"
          :disabled="disabled"
          @click="$emit('action', { type: 'detectDisplaySource' })"
        >
          重新检测
        </button>
        <button v-if="!unknown" class="text-button" @click="expanded = false">取消</button>
      </div>
    </template>
    <template v-else>
      <span>画面与记录不一致时，可重新确认。</span>
      <div class="source-actions">
        <button
          v-if="state.config.displayMethod === 'ddc'"
          class="text-button"
          :disabled="disabled"
          @click="$emit('action', { type: 'detectDisplaySource' })"
        >
          重新检测
        </button>
        <button class="text-button" :disabled="disabled" @click="expanded = true">
          确认当前画面
        </button>
      </div>
    </template>
  </div>
</template>

<style scoped>
.display-source-picker {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 12px 20px;
  margin: -8px 0 24px;
  padding: 12px 16px;
  border: 1px solid var(--line, #e1e5e1);
  border-radius: 8px;
  font-size: 12px;
}
.display-source-picker.pending {
  background: #fffaf0;
  border-color: #e5cba0;
}
.source-explanation p {
  margin: 6px 0 0;
  line-height: 1.6;
}
.source-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 12px;
  min-width: 0;
}
.source-actions .button {
  max-width: 260px;
}
.source-actions .button-label {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
</style>
