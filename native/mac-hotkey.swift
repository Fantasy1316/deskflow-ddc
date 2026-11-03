import Foundation
import CoreGraphics
import ApplicationServices

let action = CommandLine.arguments.dropFirst().first ?? "check"
if action == "screens" {
    var count: UInt32 = 0
    CGGetActiveDisplayList(0, nil, &count)
    var ids = [CGDirectDisplayID](repeating: 0, count: Int(count))
    CGGetActiveDisplayList(count, &ids, &count)
    let panels: [[String: Any]] = ids.prefix(Int(count)).map { id in
        let b = CGDisplayBounds(id)
        return ["id": String(id), "name": CGDisplayIsBuiltin(id) != 0 ? "内置屏幕" : "屏幕 \(id)", "internal": CGDisplayIsBuiltin(id) != 0,
                "bounds": ["x": b.origin.x, "y": b.origin.y, "width": b.width, "height": b.height]]
    }
    let data = try JSONSerialization.data(withJSONObject: panels)
    print(String(data: data, encoding: .utf8)!)
    exit(0)
}
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
