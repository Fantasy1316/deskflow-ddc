import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { expect, it } from 'vitest'
import FileTransferPanel from '../src/components/FileTransferPanel.vue'
import type { FileTransfer, FileTransferNetwork } from '../src/shared/transfers'
const job: FileTransfer = {
  id: 'job',
  label: '资料.zip',
  direction: 'receive',
  status: 'pending',
  peerName: 'MacBook',
  entries: 1,
  totalBytes: 6 * 1024 ** 3,
  completedBytes: 0,
  currentFile: '',
  destination: null,
  error: null,
  createdAt: '',
}
async function panel(jobs: FileTransfer[], connected: boolean, network?: FileTransferNetwork) {
  return renderToString(
    createSSRApp({
      render: () =>
        h(FileTransferPanel, {
          transfers: jobs,
          connected,
          supported: true,
          busy: false,
          preview: false,
          error: '',
          warnings: [],
          speeds: {},
          network,
        }),
    }),
  )
}
it('shows an incoming offer without starting reception before confirmation', async () => {
  const html = await panel([job], true)
  expect(html).toContain('接收并保存')
  expect(html).toContain('拒绝接收')
  expect(html).toContain('6.0 GiB')
  expect(html).not.toContain('打开接收目录')
})
it('offers opening the result only after completion and disables the single drop area offline', async () => {
  const html = await panel([{ ...job, status: 'complete', destination: '/received' }], false)
  expect(html).toContain('打开接收目录')
  expect(html).toContain('100%')
  expect(html).not.toContain('接收并保存')
  const buttons = html.match(/<button[^>]*disabled[^>]*>/g) ?? []
  expect(buttons).toHaveLength(1)
})
it('explains input protection only while this device is sending at a reduced rate', async () => {
  const network = {
    controlRttMs: 120,
    baselineRttMs: 3,
    sendLimitBytesPerSecond: 1024 * 1024,
  }
  const sending = { ...job, direction: 'send', status: 'transferring' } as FileTransfer
  expect(await panel([sending], true, network)).toContain('键鼠优先')
  expect(await panel([sending], true, { ...network, sendLimitBytesPerSecond: null })).not.toContain(
    '键鼠优先',
  )
  expect(await panel([{ ...job, status: 'transferring' }], true, network)).not.toContain('键鼠优先')
  expect(await panel([{ ...sending, status: 'paused' }], true, network)).not.toContain('键鼠优先')
})

it('requires a conflict choice and explains whole-directory replacement before completion', async () => {
  const html = await panel(
    [
      {
        ...job,
        status: 'conflict',
        completedBytes: job.totalBytes,
        conflict: { token: 'choice', name: '资料', directory: true, existingDirectory: true },
      },
    ],
    true,
  )
  expect(html).toContain('role="alertdialog"')
  expect(html).toContain('目标位置已有同名目录')
  expect(html).toContain('不会合并目录')
  expect(html).toContain('保留两者')
  expect(html).toContain('停止操作')
  expect(html).toContain('99%')
  expect(html).not.toContain('打开接收目录')
  expect(html).not.toMatch(/<button[^>]*>\s*继续接收\s*<\/button>/)
})

it('shows one clickable drag area with file and directory instructions', async () => {
  const html = await panel([], true)
  expect(html).toContain('file-dropzone')
  expect(html).toContain('点击或将文件、目录拖到此处')
  expect(html).toContain('文件夹可直接拖入')
  expect(html.match(/<button\b/g)).toHaveLength(1)
  expect(html).not.toContain('选择目录</button>')
})
