<script setup lang="ts">
import { computed, shallowRef } from 'vue'
import AppIcon from './AppIcon.vue'
import DeviceDiscovery from './DeviceDiscovery.vue'
import SettingsLock from './SettingsLock.vue'
import type { DiscoveredDevice } from '../shared/types'
import { z } from 'zod'
import { useConfigDraft } from '../composables/useConfigDraft'
import type { Action, Config, Snapshot } from '../shared/types'
import { canStartSharing } from '../shared/pairing'
const props = defineProps<{
  state: Snapshot
  busy: boolean
  connect: (config: Config, pairingCode?: string) => Promise<boolean>
}>()
const emit = defineEmits<{ action: [action: Action] }>()
const { draft, value } = useConfigDraft(() => props.state.config)
const pairingCode = shallowRef(''),
  showCode = shallowRef(false)
const dirty = computed(() => JSON.stringify(value()) !== JSON.stringify(props.state.config))
const locked = computed(() => props.busy || props.state.running)
const canStart = computed(() =>
  canStartSharing({ ...props.state, config: value() }, pairingCode.value),
)
const fallbackText = computed({
  get: () => (draft.fallbackIps ?? []).join(', '),
  set: (text: string) => {
    draft.fallbackIps = text.split(/[,，;；\s]+/).filter(Boolean)
  },
})
function chooseDevice(device: DiscoveredDevice) {
  draft.peerAddress = device.hostname
  draft.controlPort = device.port
  draft.fallbackIps = [...device.addresses]
}
const valid = computed(
  () =>
    draft.deviceName.trim().length > 0 &&
    draft.controlPort !== draft.deskflowPort &&
    z
      .array(z.string().ip())
      .max(8)
      .safeParse(draft.fallbackIps ?? []).success,
)
async function connectDevices() {
  if (props.busy || props.state.running || !valid.value || !canStart.value) return
  const success = await props.connect(
    value(),
    draft.role === 'client' ? pairingCode.value.trim() : undefined,
  )
  if (success) pairingCode.value = ''
}
</script>
<template>
  <section>
    <div class="page-heading">
      <div>
        <h1>设备与配对</h1>
        <p>选择键鼠提供端，在同一局域网中连接两台电脑。</p>
      </div>
    </div>
    <SettingsLock v-if="state.running" :busy="busy" @stop="emit('action', { type: 'stop' })" />
    <form @submit.prevent="emit('action', { type: 'save', config: value() })">
      <div class="panel">
        <div class="panel-heading">
          <span class="section-icon"><AppIcon name="laptop" /></span>
          <div>
            <h2>本机身份</h2>
            <p>名称只用于识别设备，Mac 和 Windows 都可以提供键鼠。</p>
          </div>
        </div>
        <label class="field"
          ><span>设备名称</span
          ><input
            v-model="draft.deviceName"
            maxlength="48"
            :disabled="locked"
            required
            placeholder="例如：工作电脑"
        /></label>
        <span class="field-label">键盘鼠标连接在哪里？</span>
        <div class="role-grid">
          <label :class="['role-card', { selected: draft.role === 'host', disabled: locked }]"
            ><input v-model="draft.role" type="radio" value="host" :disabled="locked" /><span
              class="role-icon"
              ><AppIcon name="keyboard" :size="24" /></span
            ><strong>连接在这台电脑</strong><small>本机提供键鼠，另一台接收控制</small
            ><span class="radio-indicator"></span></label
          ><label :class="['role-card', { selected: draft.role === 'client', disabled: locked }]"
            ><input v-model="draft.role" type="radio" value="client" :disabled="locked" /><span
              class="role-icon"
              ><AppIcon name="mouse" :size="24" /></span
            ><strong>连接在另一台电脑</strong><small>本机接收另一台电脑的键鼠控制</small
            ><span class="radio-indicator"></span
          ></label>
        </div>
        <div class="form-footer">
          <span>{{
            state.running ? '修改配置前，请先停止共享。' : dirty ? '有未保存的修改' : '配置已保存'
          }}</span
          ><button type="submit" class="button primary" :disabled="locked || !valid || !dirty">
            保存设备设置
          </button>
        </div>
      </div>
    </form>
    <details class="legacy-import">
      <summary>从 0.1 版本迁移配置</summary>
      0.2 的身份和配置独立于 0.1。可导入旧版设备设置，之后需要重新配对。
      <button
        class="text-button"
        :disabled="locked"
        @click="emit('action', { type: 'importLegacyConfig' })"
      >
        导入旧版配置
      </button>
    </details>
    <div class="panel">
      <div class="panel-heading">
        <span class="section-icon"><AppIcon name="link" /></span>
        <div>
          <h2>{{ draft.role === 'host' ? '等待另一台电脑加入' : '加入键鼠提供端' }}</h2>
          <p>
            {{
              draft.role === 'host'
                ? '在另一台电脑安装 Deskflow DDC，选择接收键鼠，再填写以下信息。'
                : state.hasSavedPairing
                  ? '已保存配对，可直接开始共享；需要连接其他电脑时，请填写新地址和配对码。'
                  : '查找并选择键鼠提供端，或手动填写主机名和备用 IP，再输入配对码。'
            }}
          </p>
        </div>
        <span v-if="state.connected" class="badge success">已连接</span>
      </div>
      <template v-if="draft.role === 'host'"
        ><div class="pairing-details">
          <div class="connection-address">
            <span class="field-label">本机主机名</span
            ><strong class="copyable-text">{{ state.hostname || '暂未获取' }}</strong
            ><span class="field-label">备用 IP</span
            ><strong class="copyable-text">{{ state.addresses.join(' / ') || '暂未获取' }}</strong
            ><small
              >控制端口 {{ state.config.controlPort }} · 键鼠端口
              {{ state.config.deskflowPort }}</small
            >
          </div>
          <div class="pairing-secret">
            <span class="field-label">配对码</span
            ><code :class="{ 'copyable-text': showCode && !state.preview }">{{
              state.preview
                ? '请在桌面 App 中生成'
                : showCode
                  ? state.pairingCode
                  : '•••••••• •••••••• ••••••••'
            }}</code>
            <div>
              <button class="text-button" @click="showCode = !showCode">
                {{ showCode ? '隐藏' : '显示' }}</button
              ><button
                class="text-button"
                :disabled="busy || state.preview"
                @click="emit('action', { type: 'copyPairing' })"
              >
                <AppIcon name="copy" :size="14" />复制</button
              ><button
                class="text-button"
                :disabled="locked || state.preview"
                @click="emit('action', { type: 'rotatePairing' })"
              >
                重新生成
              </button>
            </div>
          </div>
        </div>
        <p class="hint">配对码用于认证这两台设备。请只在你自己的另一台电脑上输入。</p></template
      >
      <template v-else>
        <DeviceDiscovery
          :devices="state.discoveredDevices ?? []"
          :searching="!!state.discovering"
          :disabled="locked"
          @refresh="emit('action', { type: 'discover' })"
          @select="chooseDevice"
        />
        <div class="two-fields">
          <label class="field"
            ><span>提供端主机名（也可填写 IP）</span
            ><input
              v-model="draft.peerAddress"
              :disabled="locked"
              placeholder="例如：工作电脑.local" /></label
          ><label class="field"
            ><span>{{ state.hasSavedPairing ? '配对码（可选）' : '配对码' }}</span
            ><input
              v-model="pairingCode"
              :disabled="locked"
              type="password"
              autocomplete="off"
              spellcheck="false"
              :placeholder="
                state.hasSavedPairing ? '已保存配对，留空复用' : '粘贴 SB1- 开头的完整配对码'
              "
          /></label>
        </div>
        <label class="field fallback-address"
          ><span>备用 IP（可选，用逗号分隔，最多 8 个）</span
          ><input
            v-model="fallbackText"
            :disabled="locked"
            placeholder="例如：192.168.1.10"
          /><small
            >每次重连会重新解析主机名；解析失败时尝试发现的地址、备用 IP 和上次已认证地址。</small
          ></label
        >
        <p v-if="dirty" class="hint">
          地址或设备设置已修改；点击下方“保存并连接”会先保存，再建立连接。
        </p></template
      >
      <div v-if="state.peer" class="paired-device">
        <span class="device-avatar"><AppIcon name="laptop" /></span>
        <div>
          <strong>{{ state.peer.name }}</strong
          ><small
            >{{ state.peer.platform === 'darwin' ? 'macOS' : 'Windows' }} ·
            {{ state.connected ? '控制连接已认证' : '当前离线' }}</small
          >
        </div>
        <AppIcon name="shield" :size="20" />
      </div>
      <p v-if="!state.running && !canStart" class="hint">
        首次配对需要填写提供端地址和完整配对码。已保存的配对可以留空复用。
      </p>
      <div class="form-footer">
        <span>{{
          state.engine === 'error'
            ? '引擎连接失败，请查看活动记录；处理后停止共享再重试。'
            : state.connected
              ? state.engine === 'ready'
                ? '键鼠引擎已连接'
                : '正在连接键鼠引擎…'
              : state.running
                ? '等待对端连接，请保持两端 App 运行。'
                : '开始前，请让显示器显示键鼠提供端的画面。'
        }}</span
        ><button
          v-if="state.running"
          class="button secondary"
          :disabled="busy"
          @click="emit('action', { type: 'stop' })"
        >
          <AppIcon name="power" :size="16" />停止共享</button
        ><button
          v-else
          class="button primary"
          :disabled="busy || !valid || !canStart"
          @click="connectDevices"
        >
          <AppIcon name="link" :size="16" />{{
            dirty
              ? '保存并连接'
              : draft.role === 'host'
                ? '开始等待连接'
                : state.hasSavedPairing
                  ? '开始共享'
                  : '配对并开始共享'
          }}
        </button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.fallback-address {
  margin-top: 20px;
}
</style>
