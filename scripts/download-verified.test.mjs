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

test('cleans partial downloads when all official download paths fail', (t) => {
  const options = fixture(t)
  let downloads = 0
  assert.throws(
    () =>
      downloadVerified(options, (tool, args) => {
        if (tool === 'gh') throw new Error('HTTP 504')
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

test('uses the official asset API when both release URLs fail', (t) => {
  const options = fixture(t)
  const apiRequests = []
  downloadVerified(options, (tool, args, execOptions) => {
    if (tool === 'curl') throw new Error('HTTP 504')
    assert.equal(tool, 'gh')
    apiRequests.push(args)
    if (apiRequests.length === 1) {
      assert.equal(args[1], 'repos/deskflow/deskflow/releases/tags/v1')
      return JSON.stringify({ assets: [{ id: 123, name: 'engine.zip' }] })
    }
    assert.equal(args[1], 'repos/deskflow/deskflow/releases/assets/123')
    assert.ok(args.includes('Accept: application/octet-stream'))
    writeFileSync(execOptions.stdio[1], payload)
  })
  assert.equal(apiRequests.length, 2)
  assert.equal(readFileSync(join(options.cache, options.name), 'utf8'), payload)
})

test('rejects corrupt content received through the API', (t) => {
  const options = fixture(t)
  assert.throws(
    () =>
      downloadVerified(options, (tool, args, execOptions) => {
        if (tool === 'curl') throw new Error('HTTP 504')
        if (args[1].includes('/tags/'))
          return JSON.stringify({ assets: [{ id: 123, name: 'engine.zip' }] })
        writeFileSync(execOptions.stdio[1], 'invalid API response')
      }),
    /Checksum mismatch/,
  )
  assert.equal(existsSync(join(options.cache, options.name)), false)
  assert.equal(existsSync(join(options.cache, options.name + '.download')), false)
})

test('requires the API metadata to contain the exact requested asset', (t) => {
  const options = fixture(t)
  assert.throws(
    () =>
      downloadVerified(options, (tool) => {
        if (tool === 'curl') throw new Error('HTTP 504')
        return JSON.stringify({ assets: [{ id: 123, name: 'other.zip' }] })
      }),
    /Official release contains no asset/,
  )
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
