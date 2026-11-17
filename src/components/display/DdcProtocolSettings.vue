<script setup lang="ts">
import { computed } from 'vue'
import type { DdcDetection, InputProtocol, ProtocolChoice } from '../../shared/types'
import { protocolName } from '../../shared/ddc'
const choice = defineModel<ProtocolChoice>('choice', { required: true })
const autoProtocol = defineModel<InputProtocol>('autoProtocol', { required: true })
const props = defineProps<{
  platform: string
  displayId: string
  detection?: DdcDetection | null
  locked: boolean
}>()
const emit = defineEmits<{
  detect: []
  apply: [protocol: InputProtocol]
  protocolChanged: [previous: InputProtocol, next: InputProtocol]
}>()
const result = computed(() =>
  props.detection?.displayId === props.displayId ? props.detection : null,
)
const current = computed(() => (choice.value === 'auto' ? autoProtocol.value : choice.value))
function apply() {
  if (!result.value?.recommended) return
  const previous = current.value
  choice.value = 'auto'
  autoProtocol.value = result.value.recommended
  emit('protocolChanged', previous, result.value.recommended)
  emit('apply', result.value.recommended)
}
function select(event: Event) {
  const previous = current.value
  const next = (event.target as HTMLSelectElement).value as ProtocolChoice
  choice.value = next
  emit('protocolChanged', previous, next === 'auto' ? autoProtocol.value : next)
}
</script>

<template>
  <div class="ddc-protocol">
    <div class="protocol-heading">
      <div>
        <h3>输入控制方式</h3>
        <p class="hint">先只读检测，再测试画面。检测不会切屏，也不会改变已保存的控制方式。</p>
      </div>
      <button
        type="button"
        class="button secondary compact"
        :disabled="locked || !displayId"
        @click="emit('detect')"
      >
        开始检测
      </button>
    </div>
    <label class="field"
      ><span>控制方式</span
      ><select :value="choice" :disabled="locked" @change="select">
        <option value="auto">自动配置（推荐）</option>
        <option value="standard">标准输入控制</option>
        <option
          v-if="platform === 'darwin' || current === 'input-alt'"
          value="input-alt"
          :disabled="platform !== 'darwin'"
        >
          兼容输入控制（部分 LG / input-alt）
        </option>
      </select></label
    >
    <p class="hint">
      当前采用：{{ protocolName(current)
      }}{{
        choice === 'auto' ? '；未检测或未采用推荐时沿用已保存设置，新安装使用标准协议。' : '。'
      }}
    </p>
    <div v-if="result" class="protocol-result" aria-live="polite">
      <strong>{{
        result.savedProfile
          ? '找到本机此前双向确认的配置'
          : result.recommended
            ? '检测到可读取的候选方式，写入仍需实测'
            : '暂时无法自动判断；读不到不代表不支持写入'
      }}</strong>
      <p v-for="reading in result.readings" :key="reading.protocol">
        {{ protocolName(reading.protocol) }}：{{
          reading.currentInput === null ? '未读到输入值' : `当前输入值 ${reading.currentInput}`
        }}
      </p>
      <div v-if="result.recommended" class="recommendation">
        <span>推荐：{{ protocolName(result.recommended) }}</span
        ><button type="button" class="text-button" :disabled="locked" @click="apply">
          使用推荐配置
        </button>
      </div>
      <p v-else class="hint">
        可手动选择控制方式与接口参考值，再测试两个方向。<template v-if="platform === 'darwin'"
          >部分 LG 型号可尝试兼容输入控制。</template
        >
      </p>
    </div>
    <p v-if="platform !== 'darwin'" class="hint">
      此端目前支持标准输入控制。另一端的协议和数值需独立配置，Mac 的兼容数值不能直接照搬。
    </p>
    <details class="protocol-advanced">
      <summary>协议说明</summary>
      <p>
        标准方式使用 VCP 0x60；兼容方式使用 input-alt（VCP
        0xF4、扩展寻址）。均需实际测试，品牌、读取成功或指令返回成功都不能单独证明可切屏。
      </p>
    </details>
  </div>
</template>

<style scoped>
.ddc-protocol {
  margin-top: 20px;
  padding-top: 18px;
  border-top: 1px solid var(--line);
}
.protocol-heading,
.recommendation {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 14px;
}
.protocol-heading h3 {
  margin: 0;
  font-size: 14px;
}
.protocol-heading .button {
  flex-shrink: 0;
}
.protocol-result {
  margin-top: 12px;
  padding: 14px 16px;
  border-radius: 10px;
  background: var(--soft);
  font-size: 12px;
  line-height: 1.7;
}
.protocol-result p {
  margin: 5px 0;
}
.protocol-advanced {
  margin-top: 12px;
  color: var(--muted);
  font-size: 12px;
  line-height: 1.7;
}
.protocol-advanced summary {
  cursor: pointer;
}
.protocol-advanced p {
  margin: 8px 0 0;
}
@media (max-width: 640px) {
  .protocol-heading {
    align-items: flex-start;
    flex-wrap: wrap;
  }
}
</style>
