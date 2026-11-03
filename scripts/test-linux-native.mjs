// Run on Ubuntu inside Xvfb. No physical input capture or display writes.
import { execFileSync, spawnSync } from 'node:child_process'
import assert from 'node:assert/strict'
const binary = 'resources/bin/linux-input'
const env = { ...process.env, XDG_SESSION_TYPE: 'x11' }
delete env.WAYLAND_DISPLAY
assert.equal(execFileSync(binary, ['check'], { env, encoding: 'utf8' }).trim(), 'true')
const panels = JSON.parse(execFileSync(binary, ['screens'], { env, encoding: 'utf8' }))
assert.ok(panels.length)
assert.ok(
  panels.every(
    (panel) =>
      /^xrandr-[a-zA-Z0-9_-]+$/.test(panel.id) && panel.id.length <= 128 && panel.bounds.width > 0,
  ),
)
const denied = spawnSync(binary, ['check'], {
  env: { ...env, XDG_SESSION_TYPE: 'wayland', WAYLAND_DISPLAY: 'wayland-0' },
  encoding: 'utf8',
})
assert.notEqual(denied.status, 0)
assert.match(denied.stderr, /Wayland/)
console.log('Linux X11 check, screen enumeration and Wayland guard passed.')
