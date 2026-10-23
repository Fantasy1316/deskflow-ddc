import { describe, expect, it } from 'vitest'
import { crossesSharedEdge } from '../electron/core/pointer-geometry'

describe('the actual built-in display seam, independent of aggregate desktop bounds', () => {
  const bounds = { x: -1512, y: 120, width: 1512, height: 982 }
  it.each([
    ['left', { x: -30, y: 500 }, { x: 150, y: 510 }],
    ['right', { x: -1490, y: 500 }, { x: -1600, y: 510 }],
    ['top', { x: -750, y: 1080 }, { x: -740, y: 1200 }],
    ['bottom', { x: -750, y: 140 }, { x: -740, y: 20 }],
  ] as const)(
    'detects a fast outward crossing with the built-in display on the %s',
    (side, previous, current) => {
      expect(crossesSharedEdge(previous, current, bounds, side)).toBe(true)
      expect(crossesSharedEdge(current, previous, bounds, side)).toBe(false)
    },
  )
  it('ignores stationary, interior, wrong-edge and diagonal corner movements', () => {
    expect(crossesSharedEdge({ x: -1, y: 500 }, { x: -1, y: 500 }, bounds, 'left')).toBe(false)
    expect(crossesSharedEdge({ x: -750, y: 500 }, { x: -700, y: 500 }, bounds, 'left')).toBe(false)
    expect(crossesSharedEdge({ x: -1490, y: 500 }, { x: -1600, y: 500 }, bounds, 'left')).toBe(
      false,
    )
    expect(crossesSharedEdge({ x: -50, y: 130 }, { x: 50, y: 20 }, bounds, 'left')).toBe(false)
  })
})
