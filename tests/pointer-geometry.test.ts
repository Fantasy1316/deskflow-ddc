import { describe, expect, it } from 'vitest'
import {
  crossesSharedEdge,
  desktopExitPoint,
  sharedEdgePosition,
} from '../electron/core/pointer-geometry'

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
  it.each([0.1, 0.5, 0.9])(
    'preserves the crossing fraction %s from a panel with an offset',
    (fraction) => {
      const y = bounds.y + fraction * (bounds.height - 1)
      const position = sharedEdgePosition({ x: -20, y }, { x: 20, y }, bounds, 'left')
      expect(position).toBeCloseTo(fraction)
      // Deskflow sees the full desktop, including the currently invisible external panel.
      expect(
        desktopExitPoint([bounds, { x: 0, y: 120, width: 1920, height: 982 }], 'left', position!),
      ).toEqual({ x: 1919, y })
    },
  )
  it.each([
    ['left', { x: 1919, y: 539.5 }],
    ['right', { x: 0, y: 539.5 }],
    ['top', { x: 959.5, y: 1079 }],
    ['bottom', { x: 959.5, y: 0 }],
  ] as const)('uses the correct outer edge with the local computer on the %s', (side, expected) => {
    expect(desktopExitPoint([{ x: 0, y: 0, width: 1920, height: 1080 }], side, 0.5)).toEqual(
      expected,
    )
  })
  it('lands on a real boundary panel when the desktop has a gap', () => {
    const displays = [
      { x: -1512, y: 0, width: 1512, height: 982 },
      { x: 0, y: 300, width: 1920, height: 1080 },
    ]
    expect(desktopExitPoint(displays, 'left', 0)).toEqual({ x: 1919, y: 300 })
    expect(() => desktopExitPoint(displays, 'left', NaN)).toThrow('跨屏位置无效')
  })
})
