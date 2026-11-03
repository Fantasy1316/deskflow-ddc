<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import AppIcon from './AppIcon.vue'
import type { Action, Snapshot, StartupPreferences } from '../shared/types'

const props = defineProps<{ state: Snapshot; busy: boolean }>()
const emit = defineEmits<{ action: [action: Action] }>()
const current = () => props.state.startup ?? { launchAtLogin: false, autoShare: false }
const draft = reactive<StartupPreferences>({ ...current() })
let baseline = { ...current() }
watch(current, (incoming) => {
  for (const key of ['launchAtLogin', 'autoShare'] as const) {
    if (draft[key] === baseline[key]) draft[key] = incoming[key]
  }
  baseline = { ...incoming }
})
const dirty = computed(
  () => draft.launchAtLogin !== current().launchAtLogin || draft.autoShare !== current().autoShare,
)
const mismatch = computed(
  () =>
    props.state.startupRegistered != null &&
    props.state.startupRegistered !== current().launchAtLogin,
)
const displayOnly = computed(() => props.state.config.mode === 'display-only')
</script>

<template>
  <form
    class="panel"
    @submit.prevent="emit('action', { type: 'saveStartup', settings: { ...draft } })"
  >
    <div class="panel-heading">
      <span class="section-icon"><AppIcon name="power" /></span>
      <div>
        <h2>启动与后台运行</h2>
        <p>两个选项独立生效，共享期间也可以修改。</p>
      </div>
    </div>
    <div class="setting-row">
      <div>
        <strong>开机自启动</strong>
        <p>登录系统后在后台启动到托盘，不自动打开主界面。</p>
      </div>
      <label class="switch-toggle">
        <input
          v-model="draft.launchAtLogin"
          type="checkbox"
          :disabled="busy"
          aria-label="开机自启动"
        />
        <span></span>
      </label>
    </div>
    <div class="setting-row">
      <div>
        <strong>启动后自动{{ displayOnly ? '启用显示器控制' : '共享' }}</strong>
        <p>
          {{
            displayOnly
              ? '应用启动时按已保存配置启用仅切屏模式，不建立网络连接。'
              : '完成配对后，应用启动时按已保存配置等待或连接另一台电脑。'
          }}
        </p>
      </div>
      <label class="switch-toggle">
        <input
          v-model="draft.autoShare"
          type="checkbox"
          :disabled="busy"
          :aria-label="displayOnly ? '启动后自动启用显示器控制' : '启动后自动共享'"
        />
        <span></span>
      </label>
    </div>
    <p class="hint">手动停止后，本次运行保持停止；权限或配置有问题时会提示处理。</p>
    <div v-if="mismatch" class="inline-info" role="status">
      系统登录启动项与保存的设置不一致，可重新保存同步；移动应用后也请重新设置。
    </div>
    <div class="form-footer">
      <span>{{
        state.startupRegistered == null
          ? '登录启动项尚未检测'
          : state.startupRegistered
            ? '系统登录启动项已启用'
            : '系统登录启动项未启用'
      }}</span>
      <button
        type="button"
        class="text-button"
        :disabled="busy"
        @click="emit('action', { type: 'checkStartup' })"
      >
        重新检测
      </button>
      <button type="submit" class="button primary" :disabled="busy || (!dirty && !mismatch)">
        保存启动设置
      </button>
    </div>
  </form>
</template>
