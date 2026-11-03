<script setup lang="ts">
import { shallowRef } from 'vue'
import AppIcon from './AppIcon.vue'
import { useFileDrop } from '../composables/useFileDrop'
import type { FileAction } from '../shared/transfers'
const props = defineProps<{ disabled: boolean; busy: boolean }>()
const emit = defineEmits<{ action: [action: FileAction] }>()
const zone = shallowRef<HTMLElement | null>(null)
const { dragging, error } = useFileDrop(
  () => zone.value,
  () => props.disabled,
  (token) => emit('action', { type: 'sendDropped', token }),
)
</script>
<template>
  <button
    ref="zone"
    type="button"
    :class="['file-dropzone', { 'is-dragging': dragging && !disabled }]"
    :disabled="disabled"
    :aria-busy="busy"
    aria-describedby="file-dropzone-hint"
    @click="emit('action', { type: 'sendFiles' })"
    @dragover.prevent
    @drop.prevent
  >
    <span class="dropzone-icon"><AppIcon name="upload" :size="32" /></span>
    <span class="dropzone-title">{{
      busy
        ? '正在处理，请稍候…'
        : dragging && !disabled
          ? '松开即可发送'
          : '点击或将文件、目录拖到此处'
    }}</span>
    <span id="file-dropzone-hint" class="dropzone-hint"
      >点击选择文件，支持多选；文件夹可直接拖入，保留完整目录结构。</span
    >
  </button>
  <p v-if="error" class="dropzone-error copyable-text" role="alert">{{ error }}</p>
</template>
<style scoped>
.file-dropzone {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  width: 100%;
  min-height: 196px;
  padding: 30px 24px;
  margin-bottom: 16px;
  border: 1px dashed #c9d7cf;
  border-radius: 12px;
  background: #f8faf8;
  color: var(--ink);
  font: inherit;
  cursor: pointer;
  transition:
    border-color 150ms ease,
    background-color 150ms ease;
}
.file-dropzone:hover:not(:disabled),
.file-dropzone.is-dragging {
  border-color: var(--green);
  background: #eff6f1;
}
.file-dropzone:focus-visible {
  outline: 2px solid var(--green);
  outline-offset: 4px;
}
.file-dropzone:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
.dropzone-icon {
  display: flex;
  color: var(--green);
}
.dropzone-title {
  font-size: 15px;
  font-weight: 600;
}
.dropzone-hint {
  color: var(--muted);
  font-size: 12px;
  line-height: 1.7;
}
.dropzone-error {
  margin-bottom: 12px;
  color: #a15d45;
  overflow-wrap: anywhere;
  font-size: 12px;
}
@media (prefers-reduced-motion: reduce) {
  .file-dropzone {
    transition: none;
  }
}
</style>
