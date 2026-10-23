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
  if (!contains(bounds, previous)) return false
  const horizontal = localSide === 'left' || localSide === 'right'
  const forward = localSide === 'left' || localSide === 'top'
  const start = horizontal ? previous.x : previous.y,
    end = horizontal ? current.x : current.y
  const origin = horizontal ? bounds.x : bounds.y,
    size = horizontal ? bounds.width : bounds.height
  const boundary = forward ? origin + size - 2 : origin + 1
  if (forward ? end <= start || end < boundary : end >= start || end > boundary) return false
  const t = Math.max(0, Math.min(1, (boundary - start) / (end - start)))
  const cross = horizontal
    ? previous.y + t * (current.y - previous.y)
    : previous.x + t * (current.x - previous.x)
  const crossOrigin = horizontal ? bounds.y : bounds.x,
    crossSize = horizontal ? bounds.height : bounds.width
  return cross >= crossOrigin && cross < crossOrigin + crossSize
}
