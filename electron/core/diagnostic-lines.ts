/** Bound diagnostic floods while preserving first occurrences and repeat counts. */
export class DiagnosticLines {
  private lines = new Map<string, number>()
  private overflow = 0
  private timer?: NodeJS.Timeout
  constructor(private emit: (line: string) => void) {}
  report(line: string) {
    // Deskflow prefixes each occurrence with its own timestamp. Preserve the
    // message while allowing otherwise identical diagnostics to coalesce.
    line = line.replace(/^\[\d{4}-\d{2}-\d{2}[T ][\d:.+Z -]+\][ \t]*/, '')
    if (!this.timer) {
      this.timer = setTimeout(() => this.flush(), 1000)
      this.timer.unref?.()
    }
    const count = this.lines.get(line)
    if (count !== undefined) {
      this.lines.set(line, count + 1)
      return
    }
    if (this.lines.size >= 20) {
      this.overflow++
      return
    }
    this.lines.set(line, 1)
    this.emit(line)
  }
  flush() {
    clearTimeout(this.timer)
    this.timer = undefined
    const lines = this.lines,
      overflow = this.overflow
    this.lines = new Map()
    this.overflow = 0
    for (const [line, count] of lines) if (count > 1) this.emit(`${line}（本秒共 ${count} 次）`)
    if (overflow) this.emit(`引擎诊断输出过多，另合并 ${overflow} 条；本秒已保留前 20 种诊断`)
  }
}
