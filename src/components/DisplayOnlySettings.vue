<script setup lang="ts">
import { computed } from 'vue'
import type { Action, Snapshot } from '../shared/types'
import { useConfigDraft } from '../composables/useConfigDraft'
import AppIcon from './AppIcon.vue'
import SettingsLock from './SettingsLock.vue'
const props = defineProps<{ state: Snapshot; busy: boolean }>()
defineEmits<{ action: [action: Action] }>()
const { draft, value } = useConfigDraft(() => props.state.config)
const dirty = computed(() => JSON.stringify(value()) !== JSON.stringify(props.state.config))
const locked = computed(() => props.busy || props.state.running)
const shortcuts = computed(() =>
  props.state.platform === 'win32'
    ? [
        ...(/\+F12$/.test(draft.shortcut) ? [draft.shortcut] : []),
        'control+alt+F10',
        'control+alt+F9',
        'control+alt+shift+F10',
      ]
    : ['control+alt+F10', 'control+alt+F9', 'control+alt+F12', 'control+alt+shift+F12'],
)
const reserved = computed(() => props.state.platform === 'win32' && /\+F12$/.test(draft.shortcut))
const label = (key: string) =>
  key
    .replace('control', 'Ctrl')
    .replace('alt', 'Alt')
    .replace('shift', 'Shift')
    .replaceAll('+', ' + ')
</script>
<template>
  <section>
    <div class="page-heading">
      <div>
        <h1>显示器切换设置</h1>
        <p>仅控制本机连接的显示器，不连接另一台电脑，不同步键鼠。</p>
      </div>
    </div>
    <SettingsLock
      v-if="state.running"
      display-only
      :busy="busy"
      @stop="$emit('action', { type: 'stop' })"
    />
    <form class="panel" @submit.prevent="$emit('action', { type: 'save', config: value() })">
      <div class="panel-heading">
        <span class="section-icon"><AppIcon name="keyboard" /></span>
        <div>
          <h2>输入源切换快捷键</h2>
          <p>启用显示器切换后，在本机按快捷键切换两个已配置输入。</p>
        </div>
      </div>
      <label class="field"
        ><span>切换两个输入源</span
        ><select v-model="draft.shortcut" :disabled="locked">
          <option
            v-for="shortcut in shortcuts"
            :key="shortcut"
            :value="shortcut"
            :disabled="state.platform === 'win32' && /\+F12$/.test(shortcut)"
          >
            {{ label(shortcut)
            }}{{
              state.platform === 'win32' && /\+F12$/.test(shortcut) ? '（Windows 保留键）' : ''
            }}
          </option>
        </select></label
      >
      <p class="hint">
        Mac 上 Alt 对应 Option，F 功能键可能需要
        Fn。若当前输入读不到，将按最近发送的指令交替；首次默认切到另一输入。显示器实体按键改变画面后，软件记录可能与实际画面不同，可先用明确目标的按钮校正。
      </p>
      <p v-if="reserved" class="validation-error">F12 是 Windows 保留键，请选择 F10 或 F9。</p>
      <div class="form-footer">
        <span>{{ dirty ? '有未保存的修改' : '保存并启用显示器切换后生效' }}</span
        ><button class="button primary" type="submit" :disabled="locked || !dirty || reserved">
          保存切换设置
        </button>
      </div>
    </form>
    <div class="panel">
      <h2>单机往返切换说明</h2>
      <p class="hint">
        两台电脑分别接入同一块显示器的不同接口。所有指令均从本机发送，部分显示器切走后不再接受本机
        DDC 指令，可能只能单向切换；请先测试两个方向，必要时用显示器实体按键恢复。
      </p>
      <p class="hint">
        本模式不会启动 Deskflow，不需要另一端运行应用；也可与独立键鼠、USB
        切换器或其他键鼠共享软件配合使用。
      </p>
    </div>
  </section>
</template>
