import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { execFileSync } from 'node:child_process'
import { patchEngine } from './patch-engine.mjs'

// Test the actual patched sendOptions body against a small native fixture;
// no network, display, physical input or installed application is used.
test('layout reloads preserve option timers; changed options and new clients still receive them', () => {
  const root = resolve(import.meta.dirname, '..')
  const dir = mkdtempSync(join(tmpdir(), 'ddc-core-test-'))
  try {
    execFileSync('tar', [
      '-xzf',
      join(root, 'src-tauri/resources/deskflow/deskflow-1.27.0-source.tar.gz'),
      '-C',
      dir,
    ])
    const source = join(dir, 'deskflow-081f6478e654a112168879e67a41ce7d7e4ae3af')
    patchEngine(source, root)
    const server = readFileSync(join(source, 'src/lib/server/Server.cpp'), 'utf8')
    const body = server.slice(
      server.indexOf('void Server::sendOptions('),
      server.indexOf('void Server::processOptions()'),
    )
    const harness = `
#include <cassert>
#include <cstdint>
#include <map>
#include <string>
#include <vector>
using OptionsList = std::vector<uint32_t>;
struct Config { using ScreenOptions = std::map<uint32_t, uint32_t>;
  ScreenOptions local, global;
  const ScreenOptions *getOptions(const std::string &name) const { return name.empty() ? &global : &local; }
};
struct BaseClientProxy { int resets=0, sets=0; OptionsList received;
  void resetOptions() { ++resets; }
  void setOptions(const OptionsList &v) { ++sets; received=v; }
};
struct Server { Config *m_config; mutable std::map<BaseClientProxy *, OptionsList> m_ddcSentOptions;
  std::string getName(BaseClientProxy *) const { return "peer"; }
  void sendOptions(BaseClientProxy *) const;
};
${body}
int main() {
  Config config; config.local[1]=10; config.global[2]=3000;
  Server server{&config,{}}; BaseClientProxy first, second;
  server.sendOptions(&first);
  for (int i=0; i<10000; ++i) server.sendOptions(&first);
  assert(first.resets==1 && first.sets==1);
  config.global[2]=4000; server.sendOptions(&first);
  assert(first.resets==2 && first.sets==2 && first.received.back()==4000);
  config.local.clear(); config.global.clear(); server.sendOptions(&first);
  assert(first.resets==3 && first.received.empty());
  server.sendOptions(&second); assert(second.resets==1);
  server.m_ddcSentOptions.erase(&first); server.sendOptions(&first);
  assert(first.resets==4); // reconnect even if the allocator reuses a pointer
}
`
    writeFileSync(join(dir, 'options.cpp'), harness)
    execFileSync('clang++', [
      '-std=c++20',
      '-Wall',
      '-Wextra',
      '-Werror',
      join(dir, 'options.cpp'),
      '-o',
      join(dir, 'options'),
    ])
    execFileSync(join(dir, 'options'))
    assert.match(server, /m_ddcSentOptions\.erase\(client\);\s+m_clients\.erase/)
    const mac = readFileSync(join(source, 'src/lib/platform/OSXComputer.mm'), 'utf8')
    for (const method of ['enter', 'leave']) {
      const body = mac.slice(mac.indexOf(`void OSXComputer::${method}()`))
      assert.match(body, /if \(m_isPrimary && m_eventTapRunLoop/)
    }
    const disable = mac.slice(
      mac.indexOf('void OSXComputer::disable()'),
      mac.indexOf('void OSXComputer::enter()'),
    )
    assert.ok(disable.indexOf('m_eventTapThread.join()') < disable.indexOf('showCursor()'))
    assert.throws(() => patchEngine(source, root), /patch context changed/)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
