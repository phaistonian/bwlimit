#!/usr/bin/swift

import AppKit
import Foundation

guard CommandLine.arguments.count == 2 else {
    fputs("usage: generate-macos-icon.swift /path/to/AppIcon.icns\n", stderr)
    exit(1)
}

let outputURL = URL(fileURLWithPath: CommandLine.arguments[1])
let fileManager = FileManager.default
let temporaryRoot = outputURL.deletingLastPathComponent()
let iconsetURL = temporaryRoot.appendingPathComponent("AppIcon.iconset", isDirectory: true)

try? fileManager.removeItem(at: iconsetURL)
try? fileManager.removeItem(at: outputURL)
try fileManager.createDirectory(at: iconsetURL, withIntermediateDirectories: true)

let sizes: [(points: Int, scale: Int)] = [
    (16, 1), (16, 2),
    (32, 1), (32, 2),
    (128, 1), (128, 2),
    (256, 1), (256, 2),
    (512, 1), (512, 2),
]

for entry in sizes {
    let pixelSize = entry.points * entry.scale
    let image = drawIcon(size: CGFloat(pixelSize))
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: pixelSize,
        pixelsHigh: pixelSize,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    )!

    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    image.draw(in: NSRect(x: 0, y: 0, width: pixelSize, height: pixelSize))
    NSGraphicsContext.restoreGraphicsState()

    let fileName: String
    if entry.scale == 1 {
        fileName = "icon_\(entry.points)x\(entry.points).png"
    } else {
        fileName = "icon_\(entry.points)x\(entry.points)@2x.png"
    }

    let pngURL = iconsetURL.appendingPathComponent(fileName)
    try rep.representation(using: .png, properties: [:])?.write(to: pngURL)
}

let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", iconsetURL.path, "-o", outputURL.path]
try process.run()
process.waitUntilExit()

guard process.terminationStatus == 0 else {
    fputs("iconutil failed\n", stderr)
    exit(process.terminationStatus)
}

try? fileManager.removeItem(at: iconsetURL)

func drawIcon(size: CGFloat) -> NSImage {
    let image = NSImage(size: NSSize(width: size, height: size))

    image.lockFocus()
    defer { image.unlockFocus() }

    NSColor.clear.setFill()
    NSBezierPath(rect: NSRect(x: 0, y: 0, width: size, height: size)).fill()

    let bounds = NSRect(x: 0, y: 0, width: size, height: size)
    let inset = size * 0.06
    let cardRect = bounds.insetBy(dx: inset, dy: inset)
    let cornerRadius = size * 0.23

    let backgroundPath = NSBezierPath(roundedRect: cardRect, xRadius: cornerRadius, yRadius: cornerRadius)

    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor(calibratedWhite: 0.0, alpha: 0.24)
    shadow.shadowBlurRadius = size * 0.05
    shadow.shadowOffset = NSSize(width: 0, height: -size * 0.02)
    shadow.set()
    backgroundPath.addClip()

    let gradient = NSGradient(colors: [
        NSColor(calibratedRed: 0.05, green: 0.45, blue: 0.47, alpha: 1.0),
        NSColor(calibratedRed: 0.06, green: 0.58, blue: 0.84, alpha: 1.0),
    ])!
    gradient.draw(in: backgroundPath, angle: -28)
    NSGraphicsContext.restoreGraphicsState()

    let glowRect = NSRect(
        x: size * 0.48,
        y: size * 0.56,
        width: size * 0.42,
        height: size * 0.28
    )
    let glowPath = NSBezierPath(ovalIn: glowRect)
    NSColor(calibratedRed: 1.0, green: 0.92, blue: 0.55, alpha: 0.22).setFill()
    glowPath.fill()

    let bubbleRect = NSRect(
        x: size * 0.18,
        y: size * 0.22,
        width: size * 0.60,
        height: size * 0.50
    )
    let bubblePath = NSBezierPath(roundedRect: bubbleRect, xRadius: size * 0.10, yRadius: size * 0.10)
    NSColor(calibratedRed: 0.97, green: 0.95, blue: 0.90, alpha: 0.97).setFill()
    bubblePath.fill()

    let tailPath = NSBezierPath()
    tailPath.move(to: NSPoint(x: size * 0.30, y: size * 0.24))
    tailPath.line(to: NSPoint(x: size * 0.23, y: size * 0.14))
    tailPath.line(to: NSPoint(x: size * 0.39, y: size * 0.22))
    tailPath.close()
    NSColor(calibratedRed: 0.97, green: 0.95, blue: 0.90, alpha: 0.97).setFill()
    tailPath.fill()

    let glyph = "Aα"
    let font = NSFont.systemFont(ofSize: size * 0.24, weight: .heavy)
    let paragraph = NSMutableParagraphStyle()
    paragraph.alignment = .center

    let attributes: [NSAttributedString.Key: Any] = [
        .font: font,
        .foregroundColor: NSColor(calibratedRed: 0.10, green: 0.20, blue: 0.29, alpha: 1.0),
        .paragraphStyle: paragraph,
    ]

    let glyphRect = NSRect(
        x: bubbleRect.minX,
        y: bubbleRect.midY - size * 0.14,
        width: bubbleRect.width,
        height: size * 0.28
    )
    glyph.draw(in: glyphRect, withAttributes: attributes)

    let underlinePath = NSBezierPath()
    underlinePath.move(to: NSPoint(x: size * 0.28, y: size * 0.33))
    underlinePath.curve(
        to: NSPoint(x: size * 0.62, y: size * 0.33),
        controlPoint1: NSPoint(x: size * 0.36, y: size * 0.29),
        controlPoint2: NSPoint(x: size * 0.53, y: size * 0.37)
    )
    underlinePath.lineWidth = size * 0.028
    underlinePath.lineCapStyle = .round
    NSColor(calibratedRed: 0.99, green: 0.56, blue: 0.38, alpha: 1.0).setStroke()
    underlinePath.stroke()

    let badgeRect = NSRect(
        x: size * 0.63,
        y: size * 0.15,
        width: size * 0.22,
        height: size * 0.22
    )
    let badgePath = NSBezierPath(ovalIn: badgeRect)
    NSColor(calibratedRed: 0.21, green: 0.80, blue: 0.61, alpha: 1.0).setFill()
    badgePath.fill()

    let checkPath = NSBezierPath()
    checkPath.move(to: NSPoint(x: badgeRect.minX + badgeRect.width * 0.26, y: badgeRect.minY + badgeRect.height * 0.53))
    checkPath.line(to: NSPoint(x: badgeRect.minX + badgeRect.width * 0.44, y: badgeRect.minY + badgeRect.height * 0.33))
    checkPath.line(to: NSPoint(x: badgeRect.minX + badgeRect.width * 0.74, y: badgeRect.minY + badgeRect.height * 0.67))
    checkPath.lineWidth = size * 0.028
    checkPath.lineCapStyle = .round
    checkPath.lineJoinStyle = .round
    NSColor.white.setStroke()
    checkPath.stroke()

    let sparkleCenter = NSPoint(x: size * 0.77, y: size * 0.74)
    let sparklePath = NSBezierPath()
    sparklePath.move(to: NSPoint(x: sparkleCenter.x, y: sparkleCenter.y + size * 0.05))
    sparklePath.line(to: NSPoint(x: sparkleCenter.x, y: sparkleCenter.y - size * 0.05))
    sparklePath.move(to: NSPoint(x: sparkleCenter.x - size * 0.05, y: sparkleCenter.y))
    sparklePath.line(to: NSPoint(x: sparkleCenter.x + size * 0.05, y: sparkleCenter.y))
    sparklePath.move(to: NSPoint(x: sparkleCenter.x - size * 0.035, y: sparkleCenter.y - size * 0.035))
    sparklePath.line(to: NSPoint(x: sparkleCenter.x + size * 0.035, y: sparkleCenter.y + size * 0.035))
    sparklePath.move(to: NSPoint(x: sparkleCenter.x - size * 0.035, y: sparkleCenter.y + size * 0.035))
    sparklePath.line(to: NSPoint(x: sparkleCenter.x + size * 0.035, y: sparkleCenter.y - size * 0.035))
    sparklePath.lineWidth = size * 0.012
    sparklePath.lineCapStyle = .round
    NSColor(calibratedRed: 1.0, green: 0.96, blue: 0.78, alpha: 0.95).setStroke()
    sparklePath.stroke()

    return image
}
