<script setup lang="ts">
import AppIcon from './AppIcon.vue'
defineProps<{ busy: boolean; displayOnly?: boolean }>()
defineEmits<{ stop: [] }>()
</script>
<template>
  <div class="settings-lock">
    <AppIcon name="shield" :size="18" /><span>{{
      displayOnly
        ? '显示器切换已启用，停用后可编辑设置。'
        : '共享运行中，设置暂不可修改。停止共享后即可编辑。'
    }}</span
    ><button class="button secondary compact" :disabled="busy" @click="$emit('stop')">
      {{ displayOnly ? '停用切换并解锁设置' : '停止共享并解锁设置' }}
    </button>
  </div>
</template>
