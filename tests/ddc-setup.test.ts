import { effectScope, nextTick, reactive, shallowRef, createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { expect, it, vi } from 'vitest'
import { defaultConfig, configSchema } from '../src/shared/config'
import { actionSchema } from '../src/shared/actions'
import { effectiveProtocol, inputPresets, savedDdcProfile } from '../src/shared/ddc'
import { useDdcSetup } from '../src/composables/useDdcSetup'
import { useConfigDraft } from '../src/composables/useConfigDraft'
import { useDisplayTest } from '../src/composables/useDisplayTest'
import DdcProtocolSettings from '../src/components/display/DdcProtocolSettings.vue'
import InputSourceField from '../src/components/InputSourceField.vue'
import type { DdcProfile, Snapshot, Action } from '../src/shared/types'

const profile: DdcProfile = {
  identity: 'stable-lg',
  name: 'LG',
  platform: 'darwin',
  protocol: 'input-alt',
  localInput: 208,
  peerInput: 144,
}
function fixture() {
  const config = { ...defaultConfig('Mac'), displayId: 'standard', localInput: 15, peerInput: 17 }
  const state = shallowRef({
    config,
    platform: 'darwin',
    ddcProfiles: [profile],
    monitors: [
      { id: 'lg', identity: 'stable-lg', name: 'LG', readable: false },
      { id: 'new', identity: 'new', name: 'LG', readable: false },
    ],
  } as Snapshot)
  const draft = reactive({ ...config })
  return { state, draft, ...useDdcSetup(() => state.value, draft) }
}
it('migrates legacy settings to standard behavior and never guesses protocol from 208', () => {
  const raw = { ...defaultConfig('Mac') } as Record<string, unknown>
  delete raw.ddcProtocol
  delete raw.ddcAutoProtocol
  const config = configSchema.parse({ ...raw, localInput: 208 })
  expect(effectiveProtocol(config)).toBe('standard')
  expect(inputPresets('input-alt').find((p) => p.name === 'DisplayPort 1')?.value).toBe(208)
  expect(inputPresets('standard').find((p) => p.name === 'DisplayPort 1')?.value).toBe(15)
  expect(
    actionSchema.safeParse({ type: 'detectDdc', displayId: 'id', command: 'set input-alt 208' })
      .success,
  ).toBe(false)
})
it('translates interface meaning on user protocol changes and clears unknown custom values', () => {
  const f = fixture()
  f.draft.ddcProtocol = 'input-alt'
  f.protocolChanged('standard', 'input-alt')
  expect([f.draft.localInput, f.draft.peerInput]).toEqual([208, 144])
  f.draft.localInput = 1234
  f.draft.ddcProtocol = 'standard'
  f.protocolChanged('input-alt', 'standard')
  expect([f.draft.localInput, f.draft.peerInput]).toEqual([null, 17])
  f.draft.localInput = 27
  f.protocolChanged('standard', 'input-alt')
  expect(f.draft.localInput).toBe(210)
  f.protocolChanged('input-alt', 'standard')
  expect(f.draft.localInput).toBe(27)
})
it('restores only the same local stable identity and clears mappings for a new display', () => {
  const f = fixture()
  f.draft.displayId = 'lg'
  f.selectDisplay()
  expect([effectiveProtocol(f.draft), f.draft.localInput, f.draft.peerInput]).toEqual([
    'input-alt',
    208,
    144,
  ])
  f.draft.displayId = 'new'
  f.selectDisplay()
  expect([effectiveProtocol(f.draft), f.draft.localInput, f.draft.peerInput]).toEqual([
    'standard',
    null,
    null,
  ])
  f.draft.displayId = 'standard'
  f.selectDisplay()
  expect([f.draft.localInput, f.draft.peerInput]).toEqual([15, 17])
  expect(
    savedDdcProfile({ ...f.state.value, platform: 'win32' }, f.state.value.monitors[0]),
  ).toBeUndefined()
  expect(savedDdcProfile(f.state.value, { id: 'lg', name: 'LG', readable: false })).toBeUndefined()
})
it('does not remap incoming saved/imported configuration through user-only handlers', async () => {
  const f = fixture(),
    scope = effectScope()
  const config = scope.run(() => useConfigDraft(() => f.state.value.config))!
  scope.run(() => useDdcSetup(() => f.state.value, config.draft))
  f.state.value = {
    ...f.state.value,
    config: {
      ...f.state.value.config,
      displayId: 'lg',
      ddcProtocol: 'input-alt',
      localInput: 208,
      peerInput: 144,
    },
  }
  await nextTick()
  expect(config.value()).toEqual(f.state.value.config)
  scope.stop()
})
it('requires separate confirmations for both directions and invalidates verification on protocol change', async () => {
  const config = shallowRef({
    ...defaultConfig('Mac'),
    displayId: 'id',
    localInput: 15,
    peerInput: 17,
  })
  const run = vi.fn(async (_a: Action) => true),
    scope = effectScope()
  const test = scope.run(() => useDisplayTest(() => config.value, run))!
  await test.remember()
  expect(run).not.toHaveBeenCalled()
  await test.test('local')
  test.confirm('local', true)
  await test.remember()
  expect(run).toHaveBeenCalledTimes(1)
  await test.test('peer')
  test.confirm('peer', true)
  expect(test.canRemember.value).toBe(false)
  await test.test('local')
  test.confirm('local', true)
  await test.remember()
  expect(run).toHaveBeenLastCalledWith({ type: 'rememberDdcProfile' })
  expect(test.remembered.value).toBe(true)
  config.value = { ...config.value, ddcAutoProtocol: 'input-alt' }
  expect(test.canRemember.value).toBe(false)
  expect(test.results.value).toEqual({ local: 'idle', peer: 'idle' })
  scope.stop()
})
it('explains read failures and scopes recommendations to the selected display and platform', async () => {
  const detection = {
    displayId: 'lg',
    identity: 'stable-lg',
    readings: [{ protocol: 'input-alt' as const, currentInput: 208, error: null }],
    recommended: 'input-alt' as const,
    savedProfile: null,
  }
  const render = (platform: string, id: string) =>
    renderToString(
      createSSRApp({
        render: () =>
          h(DdcProtocolSettings, {
            platform,
            displayId: id,
            detection,
            locked: false,
            choice: 'auto',
            autoProtocol: 'standard',
          }),
      }),
    )
  expect(await render('darwin', 'lg')).toContain('当前输入值 208')
  expect(await render('darwin', 'other')).not.toContain('当前输入值 208')
  const windows = await render('win32', 'other')
  expect(windows).not.toContain('value="input-alt"')
  expect(await render('win32', 'lg')).not.toContain('可尝试兼容输入控制')
  const field = await renderToString(
    createSSRApp({
      render: () =>
        h(InputSourceField, {
          modelValue: 208,
          disabled: false,
          name: '本机',
          protocol: 'input-alt',
        }),
    }),
  )
  expect(field).toContain('DisplayPort 1（参考值 208）')
  expect(field).not.toContain('type="number"')
})
