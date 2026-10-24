// Isolated AppKit child: no Tauri startup, prefs, providers or installed bundle.
// swift tests/native/tray-template.swift <runtime PNG> <evidence directory>
import AppKit
import Foundation

precondition(CommandLine.arguments.count == 3)
let imageURL = URL(fileURLWithPath: CommandLine.arguments[1])
let output = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
let image = NSImage(contentsOf: imageURL)!
precondition(image.representations.contains { $0.pixelsWide == 36 && $0.pixelsHigh == 36 })
image.size = NSSize(width: 18, height: 18)
image.isTemplate = true
let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
defer { NSStatusBar.system.removeStatusItem(item) }
let button = item.button!
button.image = image
precondition(button.image!.isTemplate && button.image!.size == NSSize(width: 18, height: 18))
for (mode, name) in [("light", NSAppearance.Name.aqua), ("dark", NSAppearance.Name.darkAqua)] {
    button.appearance = NSAppearance(named: name)
    button.window?.appearance = button.appearance
    button.layoutSubtreeIfNeeded()
    button.displayIfNeeded()
    for scale in [1, 2] {
        let bounds = button.bounds
        precondition(bounds.width >= 18 && bounds.height >= 18)
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil,
            pixelsWide: Int(bounds.width) * scale, pixelsHigh: Int(bounds.height) * scale,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        bitmap.size = bounds.size
        button.cacheDisplay(in: bounds, to: bitmap)
        try bitmap.representation(using: .png, properties: [:])!.write(
            to: output.appendingPathComponent("native-tray-\(mode)-\(scale)x.png"))
        print("NATIVE_TEMPLATE_PASS \(mode) \(scale)x: \(bounds.size), template=\(image.isTemplate)")
    }
}
