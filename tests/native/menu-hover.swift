// Isolated AppKit reproduction, not the installed application or a quota backend.
// swift tests/native/menu-hover.swift [--legacy] [--capture <scratch directory>]
// --legacy preserves the old enabled custom-view item and MUST fail the hover assertion.
// Moves the pointer only over this child's menu, then restores its original position.
import AppKit
import Foundation

final class Target: NSObject {
    var calls = 0
    @objc func settings(_ sender: Any?) { calls += 1 }
}
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
guard CGPreflightPostEventAccess() else {
    fputs("SKIP: event-posting permission required; no permission prompt requested\n", stderr)
    exit(77)
}
let legacy = CommandLine.arguments.contains("--legacy")
let captureIndex = CommandLine.arguments.firstIndex(of: "--capture")
let output = captureIndex.map { CommandLine.arguments[$0 + 1] }
if let output { try FileManager.default.createDirectory(atPath: output, withIntermediateDirectories: true) }
let originalMouse = NSEvent.mouseLocation
func moveMouse(_ point: NSPoint) {
    let point = CGPoint(x: point.x, y: NSScreen.screens[0].frame.maxY - point.y)
    CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point,
            mouseButton: .left)?.post(tap: .cghidEventTap)
}
var failures = 0
func check(_ value: Bool, _ message: String) {
    print("\(value ? "PASS" : "FAIL") \(message)")
    if !value { failures += 1 }
}
func makeView(_ text: String) -> NSView {
    MainActor.assumeIsolated {
        let view = NSView(frame: NSRect(x: 0, y: 0, width: 340, height: 180))
        let title = NSTextField(labelWithString: "Quota Monitor — FIXTURE")
        title.frame = NSRect(x: 16, y: 150, width: 300, height: 18)
        view.addSubview(title)
        let label = NSTextField(labelWithString: text)
        label.frame = NSRect(x: 16, y: 132, width: 300, height: 16)
        view.addSubview(label)
        return view
    }
}
let target = Target()
for (mode, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
    app.appearance = NSAppearance(named: appearance)
    for state in ["Lecture…", "Erreur", "Actualisé à 20h30"] {
        let menu = NSMenu()
        menu.autoenablesItems = false
        let panel = NSMenuItem()
        panel.view = makeView(state)
        if !legacy { panel.isEnabled = false }
        menu.addItem(panel)
        menu.addItem(.separator())
        let settings = NSMenuItem(title: "Réglages…", action: #selector(Target.settings(_:)), keyEquivalent: ",")
        settings.target = target
        menu.addItem(settings)
        let update = NSMenuItem(title: "Vérifier les mises à jour…", action: #selector(Target.settings(_:)), keyEquivalent: "")
        update.target = target
        menu.addItem(update)
        var step = 0
        var completed = false
        var warmingUp = true
        var warmupTicks = 0
        let timer = Timer(timeInterval: 0.3, repeats: true) { _ in
            guard let view = panel.view, let window = view.window else {
                warmupTicks += 1
                if warmupTicks > 30 { check(false, "child menu did not attach"); menu.cancelTracking() }
                return
            }
            // Menu tracking is not ready at a fixed delay on the first popup.
            // Establish a positive hover control before testing a negative one.
            if warmingUp {
                if menu.highlightedItem === settings {
                    warmingUp = false
                } else {
                    warmupTicks += 1
                    moveMouse(window.convertPoint(toScreen: view.convert(NSPoint(x: CGFloat(30 + warmupTicks % 2), y: -17), to: nil)))
                    if warmupTicks > 30 { check(false, "real action hover did not initialize"); menu.cancelTracking() }
                }
                return
            }
            step += 1
            if step == 1 || step == 7 {
                moveMouse(window.convertPoint(toScreen: view.convert(NSPoint(x: 4, y: 170), to: nil)))
            }
            if step == 2 || step == 8 {
                check(menu.highlightedItem !== panel, "\(mode) \(state) informational hover step=\(step)")
            }
            if step == 2, state.hasPrefix("Actualisé"), let output {
                let capture = Process()
                capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
                capture.arguments = ["-x", "-l", String(window.windowNumber), "\(output)/\(legacy ? "before" : "after")-\(mode).png"]
                do { try capture.run(); capture.waitUntilExit(); check(capture.terminationStatus == 0, "capture child window") }
                catch { check(false, "capture failed: \(error)") }
            }
            if step == 3 || step == 9 {
                // Settings is the first ordinary row below the custom panel and separator.
                moveMouse(window.convertPoint(toScreen: view.convert(NSPoint(x: 30, y: -17), to: nil)))
            }
            if step == 4 || step == 10 {
                check(menu.highlightedItem === settings, "\(mode) \(state) real action hover step=\(step)")
            }
            if step == 5 { panel.view = makeView("Actualisé à 20h31 — fixture refresh") }
            if step == 11 {
                moveMouse(window.convertPoint(toScreen: view.convert(NSPoint(x: 30, y: -42), to: nil)))
            }
            if step == 12 {
                check(menu.highlightedItem === update, "\(mode) \(state) update action hover")
                completed = true
            }
            if step == 13 { menu.cancelTracking() }
        }
        RunLoop.main.add(timer, forMode: .eventTracking)
        _ = menu.popUp(positioning: nil, at: NSPoint(x: 500, y: 500), in: nil)
        timer.invalidate()
        check(completed, "\(mode) \(state) completed all hover/refresh steps")
        let calls = target.calls
        let key = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: .command,
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: 0, context: nil,
            characters: ",", charactersIgnoringModifiers: ",", isARepeat: false, keyCode: 43)!
        check(menu.performKeyEquivalent(with: key) && target.calls == calls + 1, "\(mode) \(state) Cmd+, invokes real action")
    }
}
moveMouse(originalMouse)
print("NATIVE_HOVER_RESULT legacy=\(legacy) failures=\(failures) AppKit=\(NSAppKitVersion.current)")
exit(failures == 0 ? 0 : 1)
