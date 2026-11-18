// Apply a small, reproducible patch only to the verified Deskflow revision.
import { readFileSync, writeFileSync, copyFileSync } from 'node:fs'
import { join } from 'node:path'
export function patchEngine(source, root) {
  const edit = (name, before, after) => {
    const file = join(source, name)
    const text = readFileSync(file, 'utf8')
    if (text.split(before).length !== 2)
      throw new Error(`Pinned engine patch context changed: ${name}`)
    writeFileSync(file, text.replace(before, after))
  }
  for (const [file, target] of [
    ['primary-entry.h', 'DdcPrimaryEntry.h'],
    ['primary-route.h', 'DdcPrimaryRoute.h'],
  ])
    copyFileSync(
      join(root, 'resources/vendor/deskflow', file),
      join(source, 'src/lib/server', target),
    )
  edit(
    'src/lib/server/Server.cpp',
    '#include "server/PrimaryClient.h"',
    '#include "server/PrimaryClient.h"\n#include "server/DdcPrimaryRoute.h"',
  )
  edit(
    'src/lib/server/Server.cpp',
    '  assert(dst != nullptr);\n\n  int32_t dx;\n  int32_t dy;\n  int32_t dw;\n  int32_t dh;\n  dst->getShape(dx, dy, dw, dh);',
    `  assert(dst != nullptr);

  int32_t dx;
  int32_t dy;
  int32_t dw;
  int32_t dh;
  dst->getShape(dx, dy, dw, dh);
  // Map before both the transition record and the single native entry warp.
  if (dst == m_primaryClient && m_active != dst &&
      ddcMapPrimaryEntry(getName(m_active), dx, dy, dw, dh, x, y)) {
    LOG_INFO("ddc primary entry mapped");
  }`,
  )
  edit(
    'src/lib/server/Server.cpp',
    '    m_primaryClient->getCursorCenter(m_x, m_y);',
    `    m_primaryClient->getCursorCenter(m_x, m_y);
    int32_t ax, ay, aw, ah;
    m_primaryClient->getShape(ax, ay, aw, ah);
    ddcMapPrimaryEntry(getName(client), ax, ay, aw, ah, m_x, m_y, true);`,
  )
  edit(
    'src/lib/server/Server.h',
    '  ClientList m_clients;',
    '  ClientList m_clients;\n  mutable std::map<BaseClientProxy *, OptionsList> m_ddcSentOptions;',
  )
  edit(
    'src/lib/server/Server.cpp',
    '  m_clients.erase(getName(client));',
    '  m_ddcSentOptions.erase(client);\n  m_clients.erase(getName(client));',
  )
  edit(
    'src/lib/server/Server.cpp',
    '  // send the options\n  client->resetOptions();',
    `  // Edge-only layout reloads do not change input/heartbeat options. Avoid
  // reinitializing the receiver and its keepalive timers on every crossing.
  const auto previous = m_ddcSentOptions.find(client);
  if (previous != m_ddcSentOptions.end() && previous->second == optionsList) return;
  m_ddcSentOptions[client] = optionsList;
  // send the options
  client->resetOptions();`,
  )
  for (const method of ['enter', 'leave']) {
    edit(
      'src/lib/platform/OSXComputer.mm',
      `void OSXComputer::${method}()\n{`,
      `void OSXComputer::${method}()
{
  // Primary cursor capture must follow the queued warp on the same thread.
  // Secondary entry retains upstream ordering for keys and button resets.
  if (m_isPrimary && m_eventTapRunLoop && CFRunLoopGetCurrent() != m_eventTapRunLoop) {
    CFRunLoopPerformBlock(m_eventTapRunLoop, kCFRunLoopDefaultMode, ^{
      ${method}();
    });
    CFRunLoopWakeUp(m_eventTapRunLoop);
    return;
  }`,
    )
  }
  edit(
    'src/lib/platform/OSXComputer.mm',
    'void OSXComputer::disable()\n{\n  showCursor();',
    'void OSXComputer::disable()\n{',
  )
  edit(
    'src/lib/platform/OSXComputer.mm',
    '  if (m_eventTapThread.joinable()) {\n    m_eventTapThread.join();\n  }',
    `  if (m_eventTapThread.joinable()) {
    m_eventTapThread.join();
  }
  // A queued leave may have run while stopping the tap. Restore capture only
  // after joining it, so quit cannot leave physical input detached.
  showCursor();
  if (m_isPrimary) CGAssociateMouseAndMouseCursorPosition(true);`,
  )
  edit(
    'src/lib/server/ClientProxy1_3.cpp',
    '    // reset alarm\n    resetHeartbeatTimer();',
    '    LOG_INFO("ddc transport heartbeat received");\n    // reset alarm\n    resetHeartbeatTimer();',
  )
  edit(
    'src/lib/server/ClientProxy1_3.cpp',
    '  ProtocolUtil::writef(getStream(), kMsgCKeepAlive);',
    '  ProtocolUtil::writef(getStream(), kMsgCKeepAlive);\n  LOG_INFO("ddc transport heartbeat queued");',
  )
}
