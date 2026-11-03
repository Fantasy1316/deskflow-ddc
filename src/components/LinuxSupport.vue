<script setup lang="ts">
import type { Action, Snapshot } from '../shared/types'
defineProps<{ state: Snapshot; busy: boolean }>()
defineEmits<{ action: [action: Action] }>()
</script>

<template>
  <div v-if="state.platform === 'linux'" class="panel">
    <div class="panel-heading">
      <div>
        <h2>Ubuntu 兼容性（实验性）</h2>
        <p>完整共享需要 X11；请在登录界面选择 Ubuntu on Xorg。</p>
      </div>
    </div>
    <div class="dependency-row">
      <div>
        <strong>X11 键鼠控制</strong>
        <p>Wayland / XWayland 暂不支持键鼠共享，也不支持锁屏密码界面控制。</p>
      </div>
      <span :class="['badge', state.dependencies.accessibility ? 'success' : 'neutral']">{{
        state.dependencies.accessibility ? 'X11 输入桥可用' : '当前会话或输入桥不可用'
      }}</span>
      <button
        class="button secondary compact"
        :disabled="busy"
        @click="$emit('action', { type: 'checkAccessibility' })"
      >
        重新检测
      </button>
    </div>
    <p class="hint">
      显示器切换使用 ddcutil。请开启显示器 DDC/CI，加载 i2c-dev，并确认当前用户有 /dev/i2c-*
      访问权限；通常需要加入 i2c 组后重新登录。无需以 root 运行应用。 Wayland
      可使用“仅切换显示器”，全局快捷键可能不可用，请用界面或托盘按钮。
    </p>
  </div>
</template>
