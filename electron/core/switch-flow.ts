export interface Transition {
  from: string
  to: string
}
export interface SwitchEffects {
  writeInput(from: string, to: string): Promise<void>
  restoreKeyboard(id: string): Promise<void>
  changed(active: string | null, busy: boolean, error?: string): void
  finished(transition: Transition, error?: string): void
  failureLabel?: string
}
/** Process observed Deskflow transitions in order; never pretend a requested transition already happened. */
export class SwitchFlow {
  private chain = Promise.resolve()
  private pending = 0
  private active: string | null = null
  private rollback: Transition | null = null
  private rollbackUntil = 0
  private stopped = false
  constructor(
    private ids: Set<string>,
    private effects: SwitchEffects,
  ) {}
  observe(change: Transition) {
    if (this.stopped) return Promise.resolve()
    if (!this.ids.has(change.from) || !this.ids.has(change.to) || change.from === change.to)
      return Promise.resolve()
    if (
      Date.now() < this.rollbackUntil &&
      this.rollback?.from === change.from &&
      this.rollback.to === change.to
    ) {
      this.rollback = null
      return Promise.resolve()
    }
    this.rollback = null
    this.pending++
    this.effects.changed(this.active, true)
    const next = this.chain.then(async () => {
      if (this.stopped) {
        this.pending--
        return
      }
      try {
        await this.effects.writeInput(change.from, change.to)
        if (this.stopped) return
        this.active = change.to
        this.effects.finished(change)
      } catch (error) {
        if (this.stopped) return
        const message = error instanceof Error ? error.message : String(error)
        this.rollback = { from: change.to, to: change.from }
        this.rollbackUntil = Date.now() + 3000
        try {
          await this.effects.restoreKeyboard(change.from)
          this.active = change.from
        } catch {
          this.active = null
          this.rollback = null
        }
        if (this.stopped) return
        this.effects.changed(
          this.active,
          true,
          `${this.effects.failureLabel ?? '显示器切换失败'}：${message}。已尝试恢复键鼠控制，请检查当前画面。`,
        )
        this.effects.finished(change, message)
      } finally {
        this.pending--
        if (!this.stopped) this.effects.changed(this.active, this.pending > 0)
      }
    })
    this.chain = next.catch(() => {})
    return next
  }
  stop() {
    this.stopped = true
  }
}
