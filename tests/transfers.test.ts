import { expect, it } from 'vitest'
import {
  formatBytes,
  transferActive,
  transferProgress,
  type FileTransfer,
} from '../src/shared/transfers'
const task = (status: FileTransfer['status'], totalBytes: number, completedBytes: number) =>
  ({ status, totalBytes, completedBytes }) as FileTransfer
it('does not show completion before checksum verification and final commit', () => {
  expect(transferProgress(task('transferring', 100, 100))).toBe(99)
  expect(transferProgress(task('paused', 100, 100))).toBe(99)
  expect(transferProgress(task('complete', 100, 100))).toBe(100)
  expect(transferProgress(task('connecting', 0, 0))).toBe(0)
  expect(transferProgress(task('complete', 0, 0))).toBe(100)
})
it('keeps file sizes and progress accurate above 4 GiB', () => {
  const size = 10 * 1024 ** 3
  expect(formatBytes(size)).toBe('10.0 GiB')
  expect(transferProgress(task('transferring', size, size / 2))).toBe(50)
  expect(formatBytes(0)).toBe('0 B')
  expect(formatBytes(256 * 1024)).toBe('256 KiB')
})
it('keeps paused and completed jobs out of the active transfer state', () => {
  expect(transferActive(task('waiting', 0, 0))).toBe(true)
  expect(transferActive(task('transferring', 0, 0))).toBe(true)
  expect(transferActive(task('pending', 0, 0))).toBe(false)
  expect(transferActive(task('paused', 0, 0))).toBe(false)
  expect(transferActive(task('complete', 0, 0))).toBe(false)
})
