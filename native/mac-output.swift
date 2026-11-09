// Experimental session-only output control. No DDC, input injection or key hooks.
import Foundation
import CoreGraphics
import ApplicationServices
import Darwin

struct OutputError: Error, CustomStringConvertible {
    let description: String
    init(_ message: String) { description = message }
}
func require(_ condition: Bool, _ message: String) throws {
    if !condition { throw OutputError(message) }
}
struct OutputPanel: Codable, Equatable {
    let id: UInt32
    let uuid: String
    let internalScreen: Bool
    let active: Bool
    let mirrored: Bool
    let x: Int32
    let y: Int32
    let width: Int
    let height: Int
}
struct OutputBaseline: Codable {
    let method: String
    let target: OutputPanel
    let panels: [OutputPanel]
}
func sameUUID(_ a: String, _ b: String) -> Bool {
    a.caseInsensitiveCompare(b) == .orderedSame
}
func outputPanels() throws -> [OutputPanel] {
    var count: UInt32 = 0
    try require(CGGetOnlineDisplayList(0, nil, &count) == .success && count <= 64, "无法枚举显示输出")
    var ids = [UInt32](repeating: 0, count: Int(count))
    try require(CGGetOnlineDisplayList(count, &ids, &count) == .success, "显示输出列表发生变化，请重试")
    return ids.prefix(Int(count)).compactMap { id in
        guard let raw = CGDisplayCreateUUIDFromDisplayID(id) else { return nil }
        let bounds = CGDisplayBounds(id)
        return OutputPanel(id: id, uuid: CFUUIDCreateString(nil, raw.takeRetainedValue()) as String,
                           internalScreen: CGDisplayIsBuiltin(id) != 0, active: CGDisplayIsActive(id) != 0,
                           mirrored: CGDisplayIsInMirrorSet(id) != 0, x: Int32(bounds.minX), y: Int32(bounds.minY),
                           width: Int(bounds.width), height: Int(bounds.height))
    }
}
func captureOutput(_ uuid: String, panels: [OutputPanel]) throws -> OutputBaseline {
    guard let selected = panels.first(where: { sameUUID($0.uuid, uuid) }) else {
        throw OutputError("未找到该外接输出，请先恢复视频连接并重新扫描")
    }
    try require(selected.active && !selected.internalScreen && !selected.mirrored,
                "请选择已启用的外接屏，暂不支持内置屏或镜像输出")
    try require(panels.filter { $0.active }.count > 1, "请保留 MacBook 内置屏或另一块活动屏幕，不能断开最后一块屏幕")
    return OutputBaseline(method: "mac-native-v1", target: selected, panels: panels)
}
// A disappeared UUID is expected after disabling. Only restoration may use the
// ephemeral ID saved by the guardian in this login session, never config alone.
func resolveOutput(_ saved: OutputPanel, panels: [OutputPanel], enabled: Bool,
                   savedIDUUID: String?) throws -> UInt32 {
    if let live = panels.first(where: { sameUUID($0.uuid, saved.uuid) }) { return live.id }
    if let current = panels.first(where: { $0.id == saved.id }) {
        try require(sameUUID(current.uuid, saved.uuid), "屏幕编号已被其他显示器使用，拒绝恢复错误输出")
    }
    if let uuid = savedIDUUID {
        try require(sameUUID(uuid, saved.uuid), "保存的屏幕编号已属于其他显示器")
        return saved.id
    }
    try require(enabled, "外接输出已不可用，未再次停用")
    return saved.id
}
final class NativeOutput {
    typealias Configure = @convention(c) (OpaquePointer?, UInt32, Bool) -> Int32
    let library: UnsafeMutableRawPointer
    let configure: Configure
    init() throws {
        guard let library = dlopen("/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight", RTLD_NOW),
              let symbol = dlsym(library, "CGSConfigureDisplayEnabled") else {
            throw OutputError("系统未提供原生显示输出控制接口；请使用 DDC 或硬件切换器")
        }
        self.library = library
        configure = unsafeBitCast(symbol, to: Configure.self)
    }
    deinit { dlclose(library) }
    func set(_ baseline: OutputBaseline, enabled: Bool) throws {
        try require(baseline.method == "mac-native-v1" && !baseline.target.internalScreen && !baseline.target.mirrored,
                    "显示输出恢复数据无效")
        let panels = try outputPanels() // Initializes the fresh worker's display connection.
        // Reasserting an explicit target re-arms the guardian. A source already
        // absent/inactive needs no second disable, and must never use a stale ID.
        if !enabled && !panels.contains(where: { sameUUID($0.uuid, baseline.target.uuid) && $0.active }) { return }
        var hint: String?
        if let uuid = CGDisplayCreateUUIDFromDisplayID(baseline.target.id) {
            hint = CFUUIDCreateString(nil, uuid.takeRetainedValue()) as String
        }
        let id = try resolveOutput(baseline.target, panels: panels, enabled: enabled, savedIDUUID: hint)
        if !enabled { _ = try captureOutput(baseline.target.uuid, panels: panels) }
        var config: CGDisplayConfigRef?
        try require(CGBeginDisplayConfiguration(&config) == .success, "无法创建显示输出事务")
        let code = configure(config, id, enabled)
        guard code == 0 else {
            CGCancelDisplayConfiguration(config)
            throw OutputError("原生输出控制失败：CGSConfigureDisplayEnabled=\(code)")
        }
        let result = CGCompleteDisplayConfiguration(config, .forSession)
        try require(result == .success, "提交显示输出事务失败：\(result.rawValue)")
        if enabled {
            let until = Date().addingTimeInterval(3)
            while !(try outputPanels()).contains(where: { sameUUID($0.uuid, baseline.target.uuid) && $0.active }) {
                try require(Date() < until, "已请求启用，但原显示器 UUID 尚未恢复；请重插视频线")
                Thread.sleep(forTimeInterval: 0.1)
            }
            var layout: CGDisplayConfigRef?
            try require(CGBeginDisplayConfiguration(&layout) == .success, "输出已恢复，但无法恢复屏幕位置")
            let live = try outputPanels()
            for panel in baseline.panels where panel.active {
                if let current = live.first(where: { sameUUID($0.uuid, panel.uuid) && $0.active }) {
                    let moved = CGConfigureDisplayOrigin(layout, current.id, panel.x, panel.y)
                    if moved != .success { CGCancelDisplayConfiguration(layout); throw OutputError("输出已恢复，但恢复屏幕位置失败") }
                }
            }
            try require(CGCompleteDisplayConfiguration(layout, .forSession) == .success, "输出已恢复，但提交屏幕位置失败")
        }
    }
}
func selfTest() throws {
    let internalPanel = OutputPanel(id: 1, uuid: "internal", internalScreen: true, active: true, mirrored: false, x: 0, y: 0, width: 1440, height: 900)
    let external = OutputPanel(id: 3, uuid: "external", internalScreen: false, active: true, mirrored: false, x: 1440, y: 0, width: 1920, height: 1080)
    let saved = try captureOutput("EXTERNAL", panels: [internalPanel, external])
    try require(try resolveOutput(saved.target, panels: [internalPanel], enabled: true, savedIDUUID: nil) == 3, "Missing output must be recoverable by saved ID")
    let moved = OutputPanel(id: 9, uuid: "external", internalScreen: false, active: true, mirrored: false, x: 0, y: 0, width: 1920, height: 1080)
    try require(try resolveOutput(saved.target, panels: [internalPanel, moved], enabled: true, savedIDUUID: nil) == 9, "UUID must resolve the current ID")
    let unrelated = OutputPanel(id: 3, uuid: "other", internalScreen: false, active: true, mirrored: false, x: 0, y: 0, width: 1920, height: 1080)
    for operation: () throws -> Void in [
        { _ = try captureOutput("internal", panels: [internalPanel, external]) },
        { _ = try captureOutput("external", panels: [external]) },
        { _ = try resolveOutput(saved.target, panels: [internalPanel], enabled: false, savedIDUUID: nil) },
        { _ = try resolveOutput(saved.target, panels: [internalPanel, unrelated], enabled: true, savedIDUUID: nil) },
        { _ = try resolveOutput(saved.target, panels: [internalPanel], enabled: true, savedIDUUID: "other") }
    ] {
        var rejected = false
        do { try operation() } catch { rejected = true }
        try require(rejected, "Unsafe output selection was accepted")
    }
    let decoded = try JSONDecoder().decode(OutputBaseline.self, from: JSONEncoder().encode(saved))
    try require(decoded.target == external && decoded.panels == saved.panels, "Recovery data roundtrip failed")
}
func main() throws {
    let args = Array(CommandLine.arguments.dropFirst())
    if args == ["self-test"] { try selfTest(); print("mac-output selection and offline-ID recovery tests passed; no native calls"); return }
    guard let action = args.first else { throw OutputError("缺少显示输出操作") }
    var value: Any
    if action == "list" {
        let panels = try outputPanels()
        value = panels.map { panel in
            var reason: String?
            do { _ = try captureOutput(panel.uuid, panels: panels) }
            catch { reason = String(describing: error) }
            return ["id": panel.uuid, "cursorId": String(panel.id), "name": panel.internalScreen ? "内置屏幕" : "外接屏幕 \(panel.id)",
                    "internal": panel.internalScreen, "active": panel.active, "mirrored": panel.mirrored,
                    "unavailableReason": reason as Any? ?? NSNull(),
                    "bounds": ["x": Int(panel.x), "y": Int(panel.y), "width": panel.width, "height": panel.height]] as [String: Any]
        }
    } else if action == "capture" {
        try require(args.count == 2, "缺少外接屏 UUID")
        let baseline = try captureOutput(args[1], panels: outputPanels())
        _ = try NativeOutput()
        value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(baseline))
    } else {
        try require(["disable", "restore"].contains(action) && args.count == 2, "无效的显示输出操作")
        let baseline = try JSONDecoder().decode(OutputBaseline.self, from: Data(args[1].utf8))
        try NativeOutput().set(baseline, enabled: action == "restore")
        value = ["commandAccepted": true, "pictureVerified": false]
    }
    print(String(decoding: try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]), as: UTF8.self))
}
do { try main() } catch { fputs("\(error)\n", stderr); exit(1) }
