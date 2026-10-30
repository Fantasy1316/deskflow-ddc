<script setup lang="ts">
import type { Action, Snapshot } from '../shared/types'
defineProps<{ state: Snapshot; busy: boolean }>()
defineEmits<{ action: [action: Action] }>()
const enabled = defineModel<boolean>({ default: false })
</script>
<template>
  <div v-if="state.platform === 'win32'" class="panel">
    <div class="panel-heading">
      <div>
        <h2>锁屏与登录界面控制</h2>
        <p>Windows 接收键鼠时，可通过后台服务继续操作密码输入界面。</p>
      </div>
    </div>
    <div class="setting-row">
      <div>
        <strong>允许控制安全桌面</strong>
        <p>仅在本机作为接收端时生效；停止共享或退出后停止引擎。</p>
      </div>
      <label class="switch-toggle">
        <input
          v-model="enabled"
          type="checkbox"
          :disabled="busy || state.running || (!state.windowsService?.current && !enabled)"
          aria-label="允许控制安全桌面"
        /><span></span>
      </label>
    </div>
    <div class="dependency-row">
      <div>
        <strong>Windows 后台服务</strong>
        <p>安装、更新和移除需要管理员授权。服务不保存密码，也不会自动恢复旧共享。</p>
      </div>
      <span class="badge" :class="state.windowsService?.current ? 'success' : 'neutral'">{{
        state.windowsService?.current
          ? '服务就绪'
          : state.windowsService?.installed
            ? '需要更新'
            : '未安装'
      }}</span>
      <button
        type="button"
        class="button secondary compact"
        :disabled="busy || state.running"
        @click="$emit('action', { type: 'installWindowsService' })"
      >
        {{ state.windowsService?.installed ? '更新服务' : '安装服务' }}
      </button>
      <button
        v-if="state.windowsService?.installed"
        type="button"
        class="text-button"
        :disabled="busy || state.running"
        @click="$emit('action', { type: 'removeWindowsService' })"
      >
        移除服务
      </button>
      <button
        type="button"
        class="text-button"
        :disabled="busy"
        @click="$emit('action', { type: 'checkWindowsService' })"
      >
        重新检测
      </button>
    </div>
    <p v-if="enabled" class="inline-info">
      保存共享设置后重新开始共享。便携版删除前，请先停止共享并移除服务。
    </p>
  </div>
</template>
