import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { downloadVerified } from './download-verified.mjs'

const payload = 'verified upstream archive'
const sha256 = createHash('sha256').update(payload).digest('hex')

function fixture(t) {
  const cache = mkdtempSync(join(tmpdir(), 'ddc-download-test-'))
  t.after(() => rmSync(cache, { recursive: true, force: true }))
  return {
    cache,
    name: 'engine.zip',
    url: 'https://github.com/deskflow/deskflow/releases/download/v1/engine.zip?download=1',
    sha256,
  }
}

test('reuses only a cache entry matching the pinned digest', (t) => {
  const options = fixture(t)
  const target = join(options.cache, options.name)
  writeFileSync(target, payload)
  assert.equal(
    downloadVerified(options, () => assert.fail('verified cache must avoid the network')),
    target,
  )
  writeFileSync(target, 'damaged cached file')
  let downloads = 0
  downloadVerified(options, (_tool, args) => {
    downloads++
    writeFileSync(args.at(-1), payload)
  })
  assert.equal(downloads, 1)
  assert.equal(readFileSync(target, 'utf8'), payload)
})

test('retries a failed official request with a fresh URL and verifies before publishing', (t) => {
  const options = fixture(t)
  const addresses = []
  downloadVerified(options, (tool, args) => {
    assert.equal(tool, 'curl')
    const address = args.at(-3)
    addresses.push(address)
    if (addresses.length === 1) {
      writeFileSync(args.at(-1), 'interrupted download')
      throw new Error('HTTP 504')
    }
    assert.equal(existsSync(args.at(-1)), false)
    const fresh = new URL(address)
    const original = new URL(options.url)
    assert.equal(fresh.origin, original.origin)
    assert.equal(fresh.pathname, original.pathname)
    assert.equal(fresh.searchParams.get('download'), '1')
    assert.ok(fresh.searchParams.has('deskflow-ddc-retry'))
    writeFileSync(args.at(-1), payload)
  })
  assert.equal(addresses.length, 2)
  assert.notEqual(addresses[0], addresses[1])
  assert.equal(readFileSync(join(options.cache, options.name), 'utf8'), payload)
})

test('rejects a corrupt response without replacing a cached file or retrying its digest', (t) => {
  const options = fixture(t)
  const target = join(options.cache, options.name)
  writeFileSync(target, 'old invalid cache')
  let downloads = 0
  assert.throws(
    () =>
      downloadVerified(options, (_tool, args) => {
        downloads++
        writeFileSync(args.at(-1), 'wrong upstream archive')
      }),
    /Checksum mismatch/,
  )
  assert.equal(downloads, 1)
  assert.equal(readFileSync(target, 'utf8'), 'old invalid cache')
  assert.equal(existsSync(target + '.download'), false)
})

test('cleans partial downloads when both official requests fail', (t) => {
  const options = fixture(t)
  let downloads = 0
  assert.throws(
    () =>
      downloadVerified(options, (_tool, args) => {
        downloads++
        writeFileSync(args.at(-1), 'partial')
        throw new Error('HTTP 504')
      }),
    /HTTP 504/,
  )
  assert.equal(downloads, 2)
  assert.equal(existsSync(join(options.cache, options.name)), false)
  assert.equal(existsSync(join(options.cache, options.name + '.download')), false)
})

test('does not rewrite non-release download URLs', (t) => {
  const options = fixture(t)
  options.url = 'https://codeload.github.com/deskflow/deskflow/tar.gz/revision'
  let downloads = 0
  assert.throws(
    () =>
      downloadVerified(options, () => {
        downloads++
        throw new Error('network unavailable')
      }),
    /network unavailable/,
  )
  assert.equal(downloads, 1)
})
