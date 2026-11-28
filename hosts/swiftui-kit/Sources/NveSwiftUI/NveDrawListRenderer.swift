import Foundation
import NveHost
import SwiftUI

/// Fixed `FlatResource` (`nve.res.draw_list`) word-stream constants sourced from the generated
/// `NveHost.NveDrawList` decoder.
public enum NveDrawListConstants {
    public static let wordSize: Int = 4
    public static let headerSize: Int = NveDrawList.headerSize
    public static let maxOps: Int = Int(NveDrawList.maxDrawOps)
    public static let maxPolylinePoints: Int = Int(NveDrawList.maxPolylinePoints)
    public static let maxClipDepth: Int = Int(NveDrawList.maxClipDepth)
}

/// Decoded header of a flat `nve.res.draw_list` `u32`-word draw-list buffer.
public struct NveDrawListHeader: Equatable, Sendable {
    public let opCount: UInt32

    public init(opCount: UInt32) {
        self.opCount = opCount
    }
}

/// Zero-copy `FlatResource` (`nve.res.draw_list`) SwiftUI `GraphicsContext` renderer driven by the
/// generated `NveHost.NveDrawList` validator and balanced streaming decoder.
public enum NveDrawListRenderer {
    public static let opClear: UInt32 = NveDrawListOpcode.clear.rawValue
    public static let opFillRect: UInt32 = NveDrawListOpcode.fillRect.rawValue
    public static let opFillRoundedRect: UInt32 = NveDrawListOpcode.fillRoundedRect.rawValue
    public static let opFillCircle: UInt32 = NveDrawListOpcode.fillCircle.rawValue
    public static let opStrokeLine: UInt32 = NveDrawListOpcode.strokeLine.rawValue
    public static let opStrokePolyline: UInt32 = NveDrawListOpcode.strokePolyline.rawValue
    public static let opClipRect: UInt32 = NveDrawListOpcode.clipRect.rawValue
    public static let opRestoreClip: UInt32 = NveDrawListOpcode.restoreClip.rawValue

    /// Strictly validates a flat `nve.res.draw_list` little-endian `u32`-word stream zero-copy from `raw`.
    public static func validate(_ raw: UnsafeRawBufferPointer) throws -> NveDrawListHeader {
        if let err = NveDrawList.validate(raw) {
            throw err
        }
        return NveDrawListHeader(opCount: NveDrawList.opCount(of: raw))
    }

    /// Validates a flat `nve.res.draw_list` `Data` payload.
    public static func validate(_ data: Data) throws -> NveDrawListHeader {
        try data.withUnsafeBytes { raw in
            try validate(raw)
        }
    }

    /// Zero-copy validates and renders flat `nve.res.draw_list` word bytes onto a SwiftUI
    /// `GraphicsContext` within `bounds`.
    @discardableResult
    public static func draw(
        _ raw: UnsafeRawBufferPointer,
        into context: inout GraphicsContext,
        in bounds: CGRect,
        cornerRadius: CGFloat = 0
    ) throws -> Int {
        let header = try validate(raw)
        guard bounds.width > 0 && bounds.height > 0 else {
            return Int(header.opCount)
        }

        var baseCtx = context
        if cornerRadius > 0 {
            baseCtx.clip(to: Path(roundedRect: bounds, cornerRadius: cornerRadius))
        } else {
            baseCtx.clip(to: Path(bounds))
        }
        baseCtx.translateBy(x: bounds.minX, y: bounds.minY)
        var sink = SwiftUIDrawListSink(
            activeCtx: baseCtx,
            localBounds: CGRect(origin: .zero, size: bounds.size)
        )
        NveDrawList.streamValidated(raw, into: &sink)
        return Int(header.opCount)
    }

    /// Convenience zero-copy overload for `UnsafeBufferPointer<UInt8>` (e.g. `SceneReader` slices).
    @discardableResult
    public static func draw(
        _ bytes: UnsafeBufferPointer<UInt8>,
        into context: inout GraphicsContext,
        in bounds: CGRect,
        cornerRadius: CGFloat = 0
    ) throws -> Int {
        try draw(UnsafeRawBufferPointer(bytes), into: &context, in: bounds, cornerRadius: cornerRadius)
    }

    /// Convenience overload for `Data`.
    @discardableResult
    public static func draw(
        _ data: Data,
        into context: inout GraphicsContext,
        in bounds: CGRect,
        cornerRadius: CGFloat = 0
    ) throws -> Int {
        try data.withUnsafeBytes { raw in
            try draw(raw, into: &context, in: bounds, cornerRadius: cornerRadius)
        }
    }

}

private struct SwiftUIDrawListSink: NveDrawListSink {
    var activeCtx: GraphicsContext
    let localBounds: CGRect
    var clipStack: [GraphicsContext] = []

    mutating func onClear(color: UInt32) {
        if let c = NveSceneRenderer.color(NveColor(Rgba(packed: color))) {
            activeCtx.fill(Path(localBounds), with: .color(c))
        }
    }

    mutating func onFillRect(x: Float, y: Float, w: Float, h: Float, color: UInt32) {
        let rw = CGFloat(w)
        let rh = CGFloat(h)
        if rw > 0, rh > 0, let c = NveSceneRenderer.color(NveColor(Rgba(packed: color))) {
            activeCtx.fill(
                Path(CGRect(x: CGFloat(x), y: CGFloat(y), width: rw, height: rh)),
                with: .color(c)
            )
        }
    }

    mutating func onFillRoundedRect(
        x: Float,
        y: Float,
        w: Float,
        h: Float,
        radius: Float,
        color: UInt32
    ) {
        let rw = CGFloat(w)
        let rh = CGFloat(h)
        let r = max(0, CGFloat(radius))
        if rw > 0, rh > 0, let c = NveSceneRenderer.color(NveColor(Rgba(packed: color))) {
            let rect = CGRect(x: CGFloat(x), y: CGFloat(y), width: rw, height: rh)
            let path = r > 0 ? Path(roundedRect: rect, cornerRadius: r) : Path(rect)
            activeCtx.fill(path, with: .color(c))
        }
    }

    mutating func onFillCircle(cx: Float, cy: Float, radius: Float, color: UInt32) {
        let r = CGFloat(radius)
        if r > 0, let c = NveSceneRenderer.color(NveColor(Rgba(packed: color))) {
            let oval = CGRect(
                x: CGFloat(cx) - r,
                y: CGFloat(cy) - r,
                width: r * 2,
                height: r * 2
            )
            activeCtx.fill(Path(ellipseIn: oval), with: .color(c))
        }
    }

    mutating func onStrokeLine(
        x0: Float,
        y0: Float,
        x1: Float,
        y1: Float,
        width: Float,
        color: UInt32
    ) {
        let lw = CGFloat(width)
        if lw > 0, let c = NveSceneRenderer.color(NveColor(Rgba(packed: color))) {
            var path = Path()
            path.move(to: CGPoint(x: CGFloat(x0), y: CGFloat(y0)))
            path.addLine(to: CGPoint(x: CGFloat(x1), y: CGFloat(y1)))
            activeCtx.stroke(
                path,
                with: .color(c),
                style: StrokeStyle(lineWidth: lw, lineCap: .round, lineJoin: .round)
            )
        }
    }

    mutating func onStrokePolyline(width: Float, color: UInt32, points: NveDrawListPointSlice) {
        let lw = CGFloat(width)
        if points.count >= 2, lw > 0, let c = NveSceneRenderer.color(NveColor(Rgba(packed: color))) {
            var path = Path()
            let first = points[0]
            path.move(to: CGPoint(x: CGFloat(first.x), y: CGFloat(first.y)))
            for p in 1 ..< points.count {
                let pt = points[p]
                path.addLine(to: CGPoint(x: CGFloat(pt.x), y: CGFloat(pt.y)))
            }
            activeCtx.stroke(
                path,
                with: .color(c),
                style: StrokeStyle(lineWidth: lw, lineCap: .round, lineJoin: .round)
            )
        }
    }

    mutating func onClipRect(x: Float, y: Float, w: Float, h: Float) {
        clipStack.append(activeCtx)
        activeCtx.clip(
            to: Path(
                CGRect(
                    x: CGFloat(x),
                    y: CGFloat(y),
                    width: max(0, CGFloat(w)),
                    height: max(0, CGFloat(h))
                )
            )
        )
    }

    mutating func onRestoreClip() {
        if let previous = clipStack.popLast() {
            activeCtx = previous
        }
    }
}
