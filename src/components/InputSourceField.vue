<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
const props = defineProps<{ modelValue: number | null; disabled: boolean; name: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: number | null] }>()
const presets = [
  { value: 17, name: 'HDMI 1' },
  { value: 18, name: 'HDMI 2' },
  { value: 15, name: 'DisplayPort 1' },
  { value: 16, name: 'DisplayPort 2' },
  { value: 3, name: 'DVI 1' },
  { value: 4, name: 'DVI 2' },
  { value: 1, name: 'VGA' },
]
const custom = shallowRef(false)
watch(
  () => props.modelValue,
  () => {
    if (props.modelValue === null) custom.value = false
  },
)
const choice = computed(() =>
  custom.value || (props.modelValue !== null && !presets.some((p) => p.value === props.modelValue))
    ? 'custom'
    : props.modelValue === null
      ? ''
      : String(props.modelValue),
)
function choose(event: Event) {
  const choice = (event.target as HTMLSelectElement).value
  custom.value = choice === 'custom'
  if (choice !== 'custom') emit('update:modelValue', choice === '' ? null : Number(choice))
}
function update(event: Event) {
  const raw = (event.target as HTMLInputElement).value
  emit('update:modelValue', raw === '' ? null : Number(raw))
}
</script>
<template>
  <label class="field"
    ><span>{{ name }}的接口</span
    ><select :value="choice" :disabled="disabled" @change="choose">
      <option value="" disabled>选择接口参考值或自定义</option>
      <option v-for="preset in presets" :key="preset.value" :value="String(preset.value)">
        {{ preset.name }}（参考值 {{ preset.value }}）
      </option>
      <option value="custom">自定义数值</option>
    </select></label
  >
  <label class="field"
    ><span>{{ name }}的输入源数值</span
    ><input
      :value="modelValue ?? ''"
      type="number"
      min="0"
      max="65535"
      step="1"
      :disabled="disabled"
      placeholder="以显示器实测为准"
      @input="update"
  /></label>
</template>
