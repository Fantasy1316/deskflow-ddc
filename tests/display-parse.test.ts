import { describe, expect, it } from 'vitest'
import { parseMacDisplays } from '../electron/platform/display'
describe('macOS native monitor listing', () => {
  it('reads UUIDs and names without treating warning text as a display', () => {
    const id = '11111111-2222-4333-8444-555555555555'
    expect(
      parseMacDisplays(
        `warning [2] not a monitor\n[1] Example Display (DVI) (${id})\n[2] bad (not-a-uuid)\n`,
      ),
    ).toEqual([{ id, name: 'Example Display (DVI)', readable: false }])
    expect(parseMacDisplays('No displays found')).toEqual([])
  })
})
