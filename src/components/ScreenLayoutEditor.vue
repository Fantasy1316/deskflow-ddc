<script setup lang="ts">
import { computed, ref } from 'vue'
import AppIcon from './AppIcon.vue'
import type { ScreenSide } from '../shared/types'

const props = defineProps<{
  disabled: boolean
  readOnly: boolean
  hostName: string
  peerName: string
}>()
const side = defineModel<'off' | ScreenSide>({ required: true })
const positions = [
  { id: 'left', label: '左侧' },
  { id: 'right', label: '右侧' },
  { id: 'top', label: '上方' },
  { id: 'bottom', label: '下方' },
] as const
const stage = ref<HTMLElement>()
const dragging = ref(false)
const candidate = ref<ScreenSide>('left')
const shownSide = computed(() =>
  dragging.value ? candidate.value : side.value === 'off' ? 'left' : side.value,
)
const edge = computed(() => ({ left: '左', right: '右', top: '上', bottom: '下' })[shownSide.value])
let pointerId: number | undefined
let start: { x: number; y: number } | undefined

function choose(value: ScreenSide) {
  if (!props.disabled) side.value = value
}
function startDrag(event: PointerEvent) {
  if (props.disabled || event.button !== 0) return
  const button = event.currentTarget as HTMLElement
  pointerId = event.pointerId
  start = { x: event.clientX, y: event.clientY }
  candidate.value = shownSide.value
  button.setPointerCapture(event.pointerId)
}
function moveDrag(event: PointerEvent) {
  if (event.pointerId !== pointerId || !start || !stage.value || props.disabled) return
  if (!dragging.value && Math.hypot(event.clientX - start.x, event.clientY - start.y) < 6) return
  dragging.value = true
  const bounds = stage.value.getBoundingClientRect()
  const x = (event.clientX - bounds.left - bounds.width / 2) / bounds.width
  const y = (event.clientY - bounds.top - bounds.height / 2) / bounds.height
  candidate.value =
    Math.abs(x) > Math.abs(y) ? (x < 0 ? 'left' : 'right') : y < 0 ? 'top' : 'bottom'
}
function endDrag(event: PointerEvent) {
  if (event.pointerId !== pointerId) return
  if (dragging.value) choose(candidate.value)
  dragging.value = false
  pointerId = undefined
  start = undefined
}
function cancelDrag() {
  dragging.value = false
  pointerId = undefined
  start = undefined
}
function keyboardMove(event: KeyboardEvent) {
  const keys: Record<string, ScreenSide> = {
    ArrowLeft: 'left',
    ArrowRight: 'right',
    ArrowUp: 'top',
    ArrowDown: 'bottom',
  }
  const target = keys[event.key]
  if (target) {
    event.preventDefault()
    choose(target)
  }
}
</script>

<template>
  <div class="screen-layout-editor">
    <div
      ref="stage"
      :class="['screen-layout-stage', { disabled, dragging }]"
      aria-label="电脑屏幕布局"
    >
      <button
        v-for="position in positions"
        :key="position.id"
        type="button"
        :class="['screen-layout-slot', position.id, { target: shownSide === position.id }]"
        :disabled="disabled"
        :aria-label="`将键鼠提供端放到接收端${position.label}`"
        :aria-pressed="side === position.id"
        @click="choose(position.id)"
      >
        <AppIcon name="plus" :size="16" /><span>{{ position.label }}</span>
      </button>
      <div class="screen-layout-peer">
        <AppIcon name="monitor" :size="29" /><strong>{{ peerName }}</strong
        ><small>键鼠接收端</small>
      </div>
      <button
        type="button"
        :disabled="disabled"
        :class="['screen-layout-host', shownSide, { inactive: side === 'off' }]"
        aria-label="拖动键鼠提供端，或用方向键调整布局"
        aria-describedby="screen-layout-help"
        @pointerdown="startDrag"
        @pointermove="moveDrag"
        @pointerup="endDrag"
        @pointercancel="cancelDrag"
        @lostpointercapture="cancelDrag"
        @keydown="keyboardMove"
        @click="choose(shownSide)"
      >
        <AppIcon name="laptop" :size="27" /><strong>{{ hostName }}</strong
        ><small>{{ readOnly ? '键鼠提供端' : '键鼠提供端 · 拖动调整' }}</small>
      </button>
    </div>
    <p id="screen-layout-help" class="screen-layout-help">
      {{
        readOnly
          ? '此处显示键鼠提供端设置的电脑布局。'
          : '拖动键鼠提供端到接收端的左、右、上、下方，也可点击位置或使用方向键。'
      }}
    </p>
    <p class="screen-layout-route" role="status">
      {{
        side === 'off'
          ? readOnly
            ? '当前未启用跨屏，连接后显示提供端设置。'
            : '跨屏尚未启用，选择位置即可启用。'
          : `从接收端的${edge}边缘进入提供端，反向滑动返回接收端。`
      }}
    </p>
  </div>
</template>

<style scoped>
.screen-layout-stage {
  position: relative;
  height: 330px;
  max-width: 650px;
  margin: 0 auto;
  border: 1px solid #e2e8df;
  border-radius: 9px;
  background: #f7f9f5;
  --node-width: clamp(116px, 25%, 155px);
}
.screen-layout-slot,
.screen-layout-host,
.screen-layout-peer {
  position: absolute;
  width: var(--node-width);
  height: 88px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  flex-direction: column;
}
.screen-layout-slot {
  border: 1px dashed #cfdacd;
  background: transparent;
  color: #94a38d;
  font-size: 11px;
}
.left {
  left: 3%;
  top: 50%;
  transform: translateY(-50%);
}
.right {
  right: 3%;
  top: 50%;
  transform: translateY(-50%);
}
.top {
  left: 50%;
  top: 15px;
  transform: translateX(-50%);
}
.bottom {
  left: 50%;
  bottom: 15px;
  transform: translateX(-50%);
}
.screen-layout-peer {
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  background: #e9eee5;
  border: 1px solid #cad7c4;
  color: #68805e;
}
.screen-layout-host {
  border: 1px solid #8fa881;
  background: #edf4e7;
  color: #58734b;
  cursor: grab;
  touch-action: none;
  user-select: none;
  box-shadow: 0 2px 5px #243b2510;
}
.screen-layout-host:active {
  cursor: grabbing;
}
.screen-layout-host.inactive {
  border-style: dashed;
  background: #f5f8f1;
  color: #87987c;
}
.screen-layout-stage strong {
  font-size: 11px;
  font-weight: 550;
}
.screen-layout-stage small {
  font-size: 9px;
  opacity: 0.8;
}
.screen-layout-slot:hover:not(:disabled),
.screen-layout-slot.target {
  border-color: #8fa881;
  background: #edf4e7;
}
.screen-layout-stage button:focus-visible {
  outline: 3px solid #c5d9bd;
  outline-offset: 3px;
}
.screen-layout-stage.disabled {
  opacity: 0.65;
}
.screen-layout-stage.disabled button {
  cursor: default;
}
.screen-layout-help {
  font-size: 10px;
  color: #8c9990;
  line-height: 1.7;
  margin-top: 12px;
}
.screen-layout-route {
  font-size: 11px;
  color: #657d59;
  line-height: 1.8;
  margin: 8px 0 18px;
}
</style>
