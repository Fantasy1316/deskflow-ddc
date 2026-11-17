import { createSSRApp, h } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { expect, it } from 'vitest'
import FileSendStatus from '../src/components/FileSendStatus.vue'
import type { FileSendActivity } from '../src/composables/useFileSendPanel'

const job: FileSendActivity = {
  id: 'task',
  label: '资料文件夹',
  status: 'waiting',
  items: 3,
  totalBytes: 1024 ** 3,
  completedBytes: 512 * 1024 ** 2,
  bytesPerSecond: 20 * 1024 ** 2,
  error: null,
}
async function status(activity: FileSendActivity | null, message = '', disabled = false) {
  return renderToString(
    createSSRApp({
      render: () => h(FileSendStatus, { activity, message, error: false, disabled }),
    }),
  )
}
it('distinguishes adding a task from completion and keeps directory root counts', async () => {
  const waiting = await status(job)
  expect(waiting).toContain('已添加 3 项，等待对方接收')
  expect(waiting).not.toContain('已发送完成')
  const echoed = await status(job, '已添加 3 项，等待对方接收')
  expect(echoed.match(/已添加 3 项/g)).toHaveLength(1)
  expect(echoed).toContain(job.label)
  const complete = await status({ ...job, status: 'complete' })
  expect(complete).toContain('3 项已发送完成')
  expect(complete).not.toContain('重试')
})
it('shows real progress and speed, and caps pre-completion progress during verification', async () => {
  const sending = await status({ ...job, status: 'transferring' })
  expect(sending).toContain('50%')
  expect(sending).toContain('20.0 MiB/s')
  expect(sending).toContain('role="progressbar"')
  expect(
    await status({ ...job, status: 'transferring', completedBytes: job.totalBytes }),
  ).toContain('99%')
})
it('allows a paused task to retry and preserves actionable error text', async () => {
  const paused = { ...job, status: 'paused', error: '对端连接中断，请重新连接' } as FileSendActivity
  expect(await status(paused)).toContain('重试')
  expect(await status(paused)).toContain(paused.error)
  expect(await status(paused, '', true)).toMatch(/<button[^>]*disabled/)
  expect(await status({ ...job, status: 'cancelled' })).not.toContain('重试')
})
it('shows scan counts without inventing a task or progress before preparation completes', async () => {
  const html = await status(null, '正在整理，已发现 320 项文件与目录')
  expect(html).toContain('已发现 320 项')
  expect(html).not.toContain('progressbar')
  expect(html).not.toContain('已添加')
})
