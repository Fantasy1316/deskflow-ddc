<script setup lang="ts">
import type { DiscoveredDevice } from '../shared/types'
defineProps<{ devices: DiscoveredDevice[]; searching: boolean; disabled: boolean }>()
const emit = defineEmits<{ refresh: []; select: [device: DiscoveredDevice] }>()
</script>
<template>
  <div class="discovery-box">
    <div class="setting-row">
      <div>
        <strong>附近的键鼠提供端</strong>
        <p>提供端开始等待连接后可被发现，选择后仍需配对码认证。</p>
      </div>
      <button
        type="button"
        class="button secondary"
        :disabled="disabled || searching"
        @click="emit('refresh')"
      >
        {{ searching ? '正在查找…' : '查找设备' }}
      </button>
    </div>
    <button
      v-for="device in devices"
      :key="device.id"
      type="button"
      class="discovered-device"
      :disabled="disabled"
      @click="emit('select', device)"
    >
      <span
        ><strong>{{ device.name }}</strong
        ><small>{{ device.hostname }} · {{ device.addresses.join(' / ') }}</small></span
      >
      <span>选择</span>
    </button>
    <p v-if="!searching && !devices.length" class="hint">
      未发现设备时，可手动填写主机名和备用 IP。跨子网或禁用组播的网络可能无法发现。
    </p>
  </div>
</template>
<style scoped>
.discovery-box {
  margin-bottom: 20px;
}
.discovered-device {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  width: 100%;
  padding: 12px 14px;
  margin-top: 8px;
  border: 1px solid var(--border, #e4e7eb);
  border-radius: 10px;
  background: var(--panel, #fff);
  color: inherit;
  text-align: left;
  cursor: pointer;
}
.discovered-device small {
  display: block;
  margin-top: 4px;
  color: var(--muted, #707680);
  overflow-wrap: anywhere;
}
.discovered-device:disabled {
  opacity: 0.55;
  cursor: default;
}
</style>
