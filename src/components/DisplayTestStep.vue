<script setup lang="ts">
import { shallowRef } from 'vue'
import { useDisplayTest, type InputTarget } from '../composables/useDisplayTest'
import type { Config } from '../shared/types'
import type { ActionRunner } from '../shared/workspace'
const props = defineProps<{
  config: Config
  localName: string
  peerName: string
  disabled: boolean
  reason: string
  displayOnly?: boolean
  execute: ActionRunner
}>()
const pending = shallowRef<InputTarget | null>(null)
const { results, test, confirm, canRemember, remembered, remember } = useDisplayTest(
  () => props.config,
  props.execute,
)
const labels = {
  idle: '尚未测试',
  sending: '正在发送指令…',
  sent: '指令已发送，等待你确认画面',
  confirmed: '本页已由你确认画面正确',
  mismatch: '画面未切到预期电脑，请检查数值或连接',
  failed: '指令发送失败，请查看上方错误详情',
}
async function send() {
  const target = pending.value
  if (!target || props.disabled) return
  pending.value = null
  await test(target)
}
</script>
<template>
  <section class="panel" aria-labelledby="display-test-title">
    <div class="panel-heading">
      <span class="step-number">3</span>
      <div>
        <h2 id="display-test-title">测试并确认实际画面</h2>
        <p>
          先测试另一台输入，再测试切回本机，并分别确认画面。测试状态在当前页面有效；完成往返测试后，可记住本机配置供下次推荐。
        </p>
      </div>
    </div>
    <p class="inline-info">
      测试会改变显示器画面。请先{{
        displayOnly ? '停用显示器切换' : '停止共享'
      }}，确认两端开机、线缆已连接；若黑屏或无法切回，请用显示器实体按键恢复。
    </p>
    <p v-if="disabled" class="hint">{{ reason }}</p>
    <div class="input-test-grid">
      <div v-for="target in ['local', 'peer'] as const" :key="target" class="input-test-item">
        <h3>{{ target === 'local' ? localName : peerName }}</h3>
        <p>输入源 {{ (target === 'local' ? config.localInput : config.peerInput) ?? '未设置' }}</p>
        <span
          :class="['test-status', { confirmed: results[target] === 'confirmed' }]"
          role="status"
          >{{ labels[results[target]] }}</span
        >
        <button
          class="button secondary"
          :disabled="disabled || results[target] === 'sending'"
          @click="pending = target"
        >
          测试此输入
        </button>
        <div v-if="results[target] === 'sent' && !disabled" class="picture-confirm">
          <p>显示器现在是否显示这台电脑？</p>
          <div class="button-group">
            <button class="button secondary compact" @click="confirm(target, true)">
              是，画面正确</button
            ><button class="text-button" @click="confirm(target, false)">没有切换</button>
          </div>
        </div>
      </div>
    </div>
    <div v-if="canRemember" class="form-footer">
      <span>{{
        remembered
          ? '已记住本机显示器配置；更换线缆或接口后请重新测试'
          : '两个方向均已由你确认，可保存为这台显示器的优先推荐'
      }}</span>
      <button class="button primary" :disabled="disabled || remembered" @click="remember">
        {{ remembered ? '已记住配置' : '记住已验证配置' }}
      </button>
    </div>
    <div
      v-if="pending && !disabled"
      class="test-confirm"
      role="alertdialog"
      aria-label="确认测试显示器输入"
    >
      <span
        >即将请求显示 <b>{{ pending === 'local' ? localName : peerName }}</b
        >（输入源
        {{ pending === 'local' ? config.localInput : config.peerInput }}）。画面将立即变化。</span
      >
      <div class="button-group">
        <button class="button primary compact" @click="send">确认发送指令</button
        ><button class="text-button" @click="pending = null">取消</button>
      </div>
    </div>
  </section>
</template>
