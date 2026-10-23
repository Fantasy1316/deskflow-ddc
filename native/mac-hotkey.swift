import Foundation
import CoreGraphics
import ApplicationServices

let action = CommandLine.arguments.dropFirst().first ?? "check"
if action == "check" {
    print(AXIsProcessTrusted() ? "true" : "false")
    exit(0)
}
if action == "pointer" {
    guard CommandLine.arguments.count == 4,
          let x = Double(CommandLine.arguments[2]), let y = Double(CommandLine.arguments[3]),
          x.isFinite, y.isFinite else { exit(2) }
    guard AXIsProcessTrusted() else { fputs("需要辅助功能权限才能定位鼠标\n", stderr); exit(3) }
    let target = CGPoint(x: x, y: y)
    guard CGWarpMouseCursorPosition(target) == .success else { fputs("无法定位鼠标\n", stderr); exit(6) }
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: target, mouseButton: .left)?.post(tap: .cghidEventTap)
    print("positioned")
    exit(0)
}
guard action == "host" || action == "peer" else { fputs("Invalid shortcut target\n", stderr); exit(2) }
guard AXIsProcessTrusted() else { fputs("需要辅助功能权限才能发送切换快捷键\n", stderr); exit(3) }
let key: CGKeyCode = action == "host" ? 79 : 80 // F18 and F19
guard let source = CGEventSource(stateID: .hidSystemState),
      let down = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: true),
      let up = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: false) else { exit(4) }
let flags: CGEventFlags = [.maskControl, .maskAlternate, .maskShift]
down.flags = flags; up.flags = flags
down.post(tap: .cghidEventTap)
usleep(35000)
up.post(tap: .cghidEventTap)
print("sent")
