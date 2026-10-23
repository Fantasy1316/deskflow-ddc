import type { ChildProcess } from 'node:child_process'

/** Stop only a process we spawned. Sending a signal is not proof of exit. */
export function stopOwnedProcess(
  child: ChildProcess,
  grace = 1500,
  forceGrace = 1500,
): Promise<void> {
  if (child.exitCode != null || child.signalCode != null) return Promise.resolve()
  return new Promise((resolve, reject) => {
    let escalation: NodeJS.Timeout | undefined, deadline: NodeJS.Timeout | undefined
    const cleanup = () => {
      clearTimeout(escalation)
      clearTimeout(deadline)
      child.removeListener('exit', ended)
      child.removeListener('close', ended)
    }
    const ended = () => {
      cleanup()
      resolve()
    }
    child.once('exit', ended)
    child.once('close', ended)
    escalation = setTimeout(() => {
      deadline = setTimeout(() => {
        cleanup()
        reject(new Error('键鼠引擎仍未退出，已阻止重复启动；请导出日志并检查后台进程'))
      }, forceGrace)
      try {
        child.kill('SIGKILL')
      } catch (error) {
        cleanup()
        reject(error)
      }
    }, grace)
    try {
      child.kill('SIGTERM')
    } catch (error) {
      cleanup()
      reject(error)
    }
  })
}
