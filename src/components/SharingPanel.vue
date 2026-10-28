<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'
import PointerSettings from './PointerSettings.vue'
import ModifierSettings from './ModifierSettings.vue'
import { useConfigDraft } from '../composables/useConfigDraft'
import type { Action, Snapshot } from '../shared/types'
const props = defineProps<{ state: Snapshot; busy: boolean; checkingAccessibility: boolean }>()
const emit = defineEmits<{ action: [action: Action] }>()
const { draft, value } = useConfigDraft(() => props.state.config)
const dirty = computed(() => JSON.stringify(value()) !== JSON.stringify(props.state.config))
const locked = computed(() => props.busy || props.state.running)
const isHost = computed(() => draft.role === 'host')
const windowsHost = computed(() => props.state.platform === 'win32' && isHost.value)
const reservedShortcut = computed(() => windowsHost.value && /\+F12$/.test(draft.shortcut))
const shortcuts = computed(() =>
  windowsHost.value
    ? [
        ...(reservedShortcut.value ? [draft.shortcut] : []),
        'control+alt+F10',
        'control+alt+F9',
        'control+alt+shift+F10',
      ]
    : ['control+alt+F12', 'control+alt+F10', 'control+alt+F9', 'control+alt+shift+F12'],
)
const label = (value: string) =>
  value
    .replace('control', 'Ctrl')
    .replace('alt', 'Alt')
    .replace('shift', 'Shift')
    .replaceAll('+', ' + ')
</script>
<template>
  <section>
    <div class="page-heading">
      <div>
        <span class="eyebrow">MAKE IT YOURS</span>
        <h1>让切换符合你的习惯。</h1>
        <p>共享方式由你决定，键鼠可以连接在任意一端。</p>
      </div>
    </div>
    <form @submit.prevent="emit('action', { type: 'save', config: value() })">
      <div class="panel">
        <div class="panel-heading">
          <span class="section-icon"><AppIcon name="keyboard" /></span>
          <div>
            <h2>切换快捷键</h2>
            <p>画面与键鼠一起切换，由键鼠提供端处理全局快捷键。</p>
          </div>
        </div>
        <div v-if="!isHost" class="inline-info">
          本机是接收端，运行时使用提供端的快捷键和剪贴板设置。
        </div>
        <label class="field"
          ><span>在两台电脑之间切换</span
          ><select v-model="draft.shortcut" :disabled="locked || !isHost">
            <option
              v-for="shortcut in shortcuts"
              :key="shortcut"
              :value="shortcut"
              :disabled="windowsHost && /\+F12$/.test(shortcut)"
            >
              {{ label(shortcut)
              }}{{ windowsHost && /\+F12$/.test(shortcut) ? '（Windows 保留键）' : '' }}
            </option>
          </select></label
        >
        <div class="setting-row">
          <div>
            <strong>恢复到键鼠提供端</strong>
            <p>保留快捷键，出现异常时尝试退回提供端。</p>
          </div>
          <kbd>Ctrl + Alt + Esc</kbd>
        </div>
      </div>
      <div v-if="reservedShortcut" class="inline-info" role="alert">
        F12 是 Windows 保留键。请选择 Ctrl + Alt + F10 或 F9，保存后重新开始共享。
      </div>
      <ModifierSettings
        v-model="draft.modifierMapping"
        :locked="locked"
        :is-host="isHost"
        :connected="state.connected"
        :host-mapping="state.peer?.modifierMapping"
      />
      <PointerSettings
        v-model:side="draft.layoutSide"
        v-model:video="draft.crossScreenVideo"
        v-model:retained="draft.retainedDisplayId"
        :locked="locked"
        :is-host="isHost"
        :state="state"
      />
      <div class="panel">
        <div class="panel-heading">
          <span class="section-icon"><AppIcon name="clipboard" /></span>
          <div>
            <h2>剪贴板共享</h2>
            <p>由 Deskflow 在设备之间同步支持的文字、图片等格式。</p>
          </div>
        </div>
        <div class="setting-row">
          <div>
            <strong>启用跨设备复制与粘贴</strong>
            <p>与键鼠连接一起启用，App 不记录剪贴板内容。</p>
          </div>
          <label class="switch-toggle"
            ><input
              v-model="draft.clipboard"
              type="checkbox"
              :disabled="locked || !isHost"
              aria-label="启用剪贴板共享" /><span></span
          ></label>
        </div>
        <label class="field small-field"
          ><span>单次剪贴板传输上限</span
          ><select
            v-model.number="draft.clipboardMegabytes"
            :disabled="locked || !isHost || !draft.clipboard"
          >
            <option :value="1">1 MB</option>
            <option :value="4">4 MB</option>
            <option :value="8">8 MB</option>
            <option :value="16">16 MB</option>
            <option :value="32">32 MB</option>
          </select></label
        >
      </div>
      <div class="panel">
        <div class="panel-heading">
          <span class="section-icon"><AppIcon name="wifi" /></span>
          <div>
            <h2>局域网连接</h2>
            <p>同一局域网直连，两端控制端口需要一致。</p>
          </div>
        </div>
        <div class="two-fields">
          <label class="field"
            ><span>控制连接端口</span
            ><input
              v-model.number="draft.controlPort"
              type="number"
              min="1024"
              max="65535"
              required
              :disabled="locked" /></label
          ><label class="field"
            ><span>键鼠共享端口</span
            ><input
              v-model.number="draft.deskflowPort"
              type="number"
              min="1024"
              max="65535"
              required
              :disabled="locked"
          /></label>
        </div>
        <div class="form-footer">
          <span>{{
            state.running
              ? '请先停止共享再修改设置。'
              : dirty
                ? '有未保存的修改'
                : '使用加密控制连接与 Deskflow TLS'
          }}</span
          ><button class="button primary" type="submit" :disabled="locked || !dirty">
            保存共享设置
          </button>
        </div>
      </div>
    </form>
    <div class="panel">
      <div class="panel-heading">
        <span class="section-icon"><AppIcon name="cable" /></span>
        <div>
          <h2>本机运行环境</h2>
          <p>
            已内置 Deskflow 1.27.0，无需另行安装。开始共享前请退出其他 Deskflow 实例及旧版共享。
          </p>
        </div>
      </div>
      <div class="dependency-row">
        <div>
          <strong>键鼠与剪贴板引擎</strong>
          <p :class="{ 'copyable-text': !!state.config.deskflowPath }">
            {{ state.config.deskflowPath || '使用内置 Deskflow 1.27.0（离线可用）' }}
          </p>
        </div>
        <span :class="['badge', state.dependencies.deskflow ? 'success' : 'neutral']">{{
          state.dependencies.deskflow
            ? state.engineSource === 'external'
              ? '外部引擎'
              : '内置就绪'
            : '引擎未准备'
        }}</span
        ><button
          class="button secondary compact"
          :disabled="locked"
          @click="emit('action', { type: 'chooseDeskflow' })"
        >
          选择程序
        </button>
      </div>
      <button
        v-if="state.config.deskflowPath"
        class="text-button"
        :disabled="locked"
        @click="emit('action', { type: 'useBundledEngine' })"
      >
        恢复使用内置引擎
      </button>
      <div class="dependency-row">
        <div>
          <strong>显示器控制后端</strong>
          <p>
            {{
              state.platform === 'win32' ? 'Windows DXVA2 / DDC/CI' : 'Apple Silicon 原生 DDC/CI'
            }}
          </p>
        </div>
        <span :class="['badge', state.dependencies.displayControl ? 'success' : 'neutral']">{{
          state.dependencies.displayControl ? '已就绪' : '桌面版检测'
        }}</span>
      </div>
      <div v-if="state.platform === 'darwin'" class="dependency-row">
        <div>
          <strong>辅助功能权限</strong>
          <p>检测本应用的切换权限，返回应用时自动刷新；Deskflow 的键鼠共享权限需单独开启。</p>
        </div>
        <span :class="['badge', state.dependencies.accessibility ? 'success' : 'neutral']">{{
          state.dependencies.accessibility ? '已授权' : '未检测到授权'
        }}</span
        ><button
          class="button secondary compact"
          :disabled="busy"
          :aria-busy="checkingAccessibility"
          @click="emit('action', { type: 'checkAccessibility' })"
        >
          {{ checkingAccessibility ? '检测中…' : '重新检测' }}</button
        ><button
          class="button secondary compact"
          :disabled="busy"
          @click="emit('action', { type: 'openAccessibility' })"
        >
          打开系统设置
        </button>
      </div>
    </div>
  </section>
</template>
