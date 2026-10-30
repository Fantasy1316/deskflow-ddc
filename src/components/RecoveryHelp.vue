<script setup lang="ts">
import AppIcon from './AppIcon.vue'
defineProps<{ running: boolean; busy: boolean; displayOnly?: boolean }>()
defineEmits<{ stop: [] }>()
</script>
<template>
  <details class="recovery-help">
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
