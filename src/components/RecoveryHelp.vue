<script setup lang="ts">
import { onMounted, onUnmounted, useTemplateRef } from 'vue'
import AppIcon from './AppIcon.vue'
defineProps<{ running: boolean; busy: boolean; displayOnly?: boolean }>()
defineEmits<{ stop: [] }>()
const help = useTemplateRef<HTMLDetailsElement>('help')
function close() {
  if (help.value) help.value.open = false
}
function closeOutside(event: Event) {
  if (event.target instanceof Node && !help.value?.contains(event.target)) close()
}
onMounted(() => {
  document.addEventListener('pointerdown', closeOutside, true)
  document.addEventListener('focusin', closeOutside, true)
  window.addEventListener('blur', close)
})
onUnmounted(() => {
  document.removeEventListener('pointerdown', closeOutside, true)
  document.removeEventListener('focusin', closeOutside, true)
  window.removeEventListener('blur', close)
})
</script>
<template>
  <details ref="help" class="recovery-help" @keydown.esc.stop="close">
    <summary>恢复帮助</summary>
    <div class="recovery-popover">
      <strong>{{ displayOnly ? '画面切换异常？' : '键鼠或画面切换异常？' }}</strong>
      <p v-if="!displayOnly">
        可尝试 <kbd>Ctrl + Alt + Esc</kbd> 将键鼠恢复到提供端。Mac 上 Alt 对应 Option。
      </p>
      <p>
        画面黑屏或无法切回时，可使用显示器实体按键选择输入源。{{
          displayOnly
            ? '部分显示器仅接受当前活动输入端的 DDC 指令。'
            : '键鼠仍无法恢复时，先停止共享。'
        }}
      </p>
      <button
        v-if="running"
        class="button secondary compact"
        :disabled="busy"
        @click="$emit('stop')"
      >
        <AppIcon name="power" :size="15" />{{ displayOnly ? '停用显示器切换' : '停止共享' }}
      </button>
    </div>
  </details>
</template>
