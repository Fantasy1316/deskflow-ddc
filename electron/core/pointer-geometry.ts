import type { ScreenSide } from '../../src/shared/types.js'

export interface Point {
  x: number
  y: number
}
export interface Bounds extends Point {
  width: number
  height: number
}
export function contains(bounds: Bounds, point: Point) {
  return (
    point.x >= bounds.x &&
    point.x < bounds.x + bounds.width &&
    point.y >= bounds.y &&
    point.y < bounds.y + bounds.height
  )
}
/** Cross only the selected display's edge toward the shared physical monitor. */
export function crossesSharedEdge(
  previous: Point,
  current: Point,
  bounds: Bounds,
  localSide: ScreenSide,
) {
  return sharedEdgePosition(previous, current, bounds, localSide) !== undefined
}

/** Fraction along the seam, rather than a target computer's remembered jump position. */
export function sharedEdgePosition(
  previous: Point,
  current: Point,
  bounds: Bounds,
  localSide: ScreenSide,
): number | undefined {
  if (!contains(bounds, previous)) return
  const horizontal = localSide === 'left' || localSide === 'right'
  const forward = localSide === 'left' || localSide === 'top'
  const start = horizontal ? previous.x : previous.y,
    end = horizontal ? current.x : current.y
  const origin = horizontal ? bounds.x : bounds.y,
    size = horizontal ? bounds.width : bounds.height
  const boundary = forward ? origin + size - 2 : origin + 1
  if (forward ? end <= start || end < boundary : end >= start || end > boundary) return
  const t = Math.max(0, Math.min(1, (boundary - start) / (end - start)))
  const cross = horizontal
    ? previous.y + t * (current.y - previous.y)
    : previous.x + t * (current.x - previous.x)
  const crossOrigin = horizontal ? bounds.y : bounds.x,
    crossSize = horizontal ? bounds.height : bounds.width
  if (cross >= crossOrigin && cross < crossOrigin + crossSize)
    return Math.max(0, Math.min(1, (cross - crossOrigin) / Math.max(1, crossSize - 1)))
}

/** Reach Deskflow's real desktop boundary so it maps and owns the destination coordinates. */
export function desktopExitPoint(
  displays: Bounds[],
  localSide: ScreenSide,
  position: number,
): Point {
  if (!displays.length || !Number.isFinite(position) || position < 0 || position > 1)
    throw new Error('跨屏位置无效')
  const horizontal = localSide === 'left' || localSide === 'right'
  const forward = localSide === 'left' || localSide === 'top'
  const start = Math.min(...displays.map((b) => (horizontal ? b.x : b.y)))
  const end = Math.max(...displays.map((b) => (horizontal ? b.x + b.width : b.y + b.height)))
  const crossStart = Math.min(...displays.map((b) => (horizontal ? b.y : b.x)))
  const crossEnd = Math.max(...displays.map((b) => (horizontal ? b.y + b.height : b.x + b.width)))
  const edge = forward ? end - 1 : start
  const desired = crossStart + position * Math.max(0, crossEnd - crossStart - 1)
  // A rectangular desktop can include gaps. Use a real panel on the outer edge.
  const candidates = displays.filter((b) =>
    horizontal ? edge >= b.x && edge < b.x + b.width : edge >= b.y && edge < b.y + b.height,
  )
  const cross = candidates
    .map((b) => {
      const origin = horizontal ? b.y : b.x
      const size = horizontal ? b.height : b.width
      return Math.max(origin, Math.min(desired, origin + size - 1))
    })
    .sort((a, b) => Math.abs(a - desired) - Math.abs(b - desired))[0]!
  return horizontal ? { x: edge, y: cross } : { x: cross, y: edge }
}
