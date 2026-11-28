import NveHost
import Foundation
import SwiftUI

/// Fixed `FlatResource` (`nve.res.draw_list`) word-stream constants matching
/// `packages/modules/canvas/src/flat.rs`.
public enum NveDrawListConstants {
    public static let wordSize: Int = 4
    public static let headerSize: Int = 4
    public static let maxOps: Int = 65_536
    public static let maxPolylinePoints: Int = 16_384
    public static let maxClipDepth: Int = 64
}

/// Decoded header of a flat `nve.res.draw_list` `u32`-word draw-list buffer.
public struct NveDrawListHeader: Equatable, Sendable {
    public let opCount: UInt32

    public init(opCount: UInt32) {
        self.opCount = opCount
    }
}

/// Strict validation and decode errors for flat `nve.res.draw_list` word streams.
public enum NveDrawListError: Error, Equatable, Sendable {
    case truncated
    case opCountExceedsLimit(UInt32)
    case polylinePointCountExceedsLimit(UInt32)
    case unknownOpcode(UInt32)
    case nonFiniteFloat
    case trailingBytes(Int)
}

/// Zero-copy `FlatResource` (`nve.res.draw_list`) binary vector draw-list validator & SwiftUI
/// `GraphicsContext` renderer matching `packages/modules/canvas/src/flat.rs` byte-for-byte.
///
/// Decodes all 8 opcodes (`Clear`, `FillRect`, `FillRoundedRect`, `FillCircle`, `StrokeLine`,
/// `StrokePolyline`, `ClipRect`, `RestoreClip`) from little-endian `u32` words directly out of
/// `UnsafeRawBufferPointer` or `Data` with strict bounds, NaN/Inf, vertex-cap, and balanced
/// clip-stack guards.
public enum NveDrawListRenderer {
    public static let opClear: UInt32 = 1
    public static let opFillRect: UInt32 = 2
    public static let opFillRoundedRect: UInt32 = 3
    public static let opFillCircle: UInt32 = 4
    public static let opStrokeLine: UInt32 = 5
    public static let opStrokePolyline: UInt32 = 6
    public static let opClipRect: UInt32 = 7
    public static let opRestoreClip: UInt32 = 8

    /// Strictly validates a flat `nve.res.draw_list` little-endian `u32`-word stream zero-copy from `raw`.
    public static func validate(_ raw: UnsafeRawBufferPointer) throws -> NveDrawListHeader {
        let len = raw.count
        guard len >= NveDrawListConstants.headerSize else {
            throw NveDrawListError.truncated
        }

        @inline(__always)
        func loadU32(_ offset: Int) -> UInt32 {
            UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        }

        @inline(__always)
        func checkFiniteFloat(at offset: Int) throws {
            let v = Float(bitPattern: loadU32(offset))
            if !v.isFinite {
                throw NveDrawListError.nonFiniteFloat
            }
        }

        let opCount = loadU32(0)
        guard opCount <= UInt32(NveDrawListConstants.maxOps) else {
            throw NveDrawListError.opCountExceedsLimit(opCount)
        }

        var pos = NveDrawListConstants.headerSize
        guard len - pos >= Int(opCount) * NveDrawListConstants.wordSize else {
            throw NveDrawListError.truncated
        }

        for _ in 0 ..< Int(opCount) {
            guard len - pos >= NveDrawListConstants.wordSize else {
                throw NveDrawListError.truncated
            }
            let opcode = loadU32(pos)
            pos += NveDrawListConstants.wordSize

            switch opcode {
            case opClear:
                guard len - pos >= 4 else { throw NveDrawListError.truncated }
                pos += 4

            case opFillRect:
                guard len - pos >= 20 else { throw NveDrawListError.truncated }
                try checkFiniteFloat(at: pos)
                try checkFiniteFloat(at: pos + 4)
                try checkFiniteFloat(at: pos + 8)
                try checkFiniteFloat(at: pos + 12)
                pos += 20

            case opFillRoundedRect:
                guard len - pos >= 24 else { throw NveDrawListError.truncated }
                try checkFiniteFloat(at: pos)
                try checkFiniteFloat(at: pos + 4)
                try checkFiniteFloat(at: pos + 8)
                try checkFiniteFloat(at: pos + 12)
                try checkFiniteFloat(at: pos + 16)
                pos += 24

            case opFillCircle:
                guard len - pos >= 16 else { throw NveDrawListError.truncated }
                try checkFiniteFloat(at: pos)
                try checkFiniteFloat(at: pos + 4)
                try checkFiniteFloat(at: pos + 8)
                pos += 16

            case opStrokeLine:
                guard len - pos >= 24 else { throw NveDrawListError.truncated }
                try checkFiniteFloat(at: pos)
                try checkFiniteFloat(at: pos + 4)
                try checkFiniteFloat(at: pos + 8)
                try checkFiniteFloat(at: pos + 12)
                try checkFiniteFloat(at: pos + 16)
                pos += 24

            case opStrokePolyline:
                guard len - pos >= 12 else { throw NveDrawListError.truncated }
                try checkFiniteFloat(at: pos)
                let ptCount = loadU32(pos + 8)
                pos += 12
                guard ptCount <= UInt32(NveDrawListConstants.maxPolylinePoints) else {
                    throw NveDrawListError.polylinePointCountExceedsLimit(ptCount)
                }
                let needed = Int(ptCount) * 8
                guard len - pos >= needed else {
                    throw NveDrawListError.truncated
                }
                for _ in 0 ..< Int(ptCount) {
                    try checkFiniteFloat(at: pos)
                    try checkFiniteFloat(at: pos + 4)
                    pos += 8
                }

            case opClipRect:
                guard len - pos >= 16 else { throw NveDrawListError.truncated }
                try checkFiniteFloat(at: pos)
                try checkFiniteFloat(at: pos + 4)
                try checkFiniteFloat(at: pos + 8)
                try checkFiniteFloat(at: pos + 12)
                pos += 16

            case opRestoreClip:
                break

            default:
                throw NveDrawListError.unknownOpcode(opcode)
            }
        }

        if pos != len {
            throw NveDrawListError.trailingBytes(len - pos)
        }
        return NveDrawListHeader(opCount: opCount)
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
        let localBounds = CGRect(origin: .zero, size: bounds.size)

        var activeCtx = baseCtx
        var clipStack: [GraphicsContext] = []
        var overflowClipDepth = 0
        var pos = NveDrawListConstants.headerSize

        @inline(__always)
        func loadU32(_ offset: Int) -> UInt32 {
            UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        }

        @inline(__always)
        func loadF32(_ offset: Int) -> CGFloat {
            CGFloat(Float(bitPattern: loadU32(offset)))
        }

        for _ in 0 ..< Int(header.opCount) {
            let opcode = loadU32(pos)
            pos += NveDrawListConstants.wordSize

            switch opcode {
            case opClear:
                let rgba = loadU32(pos)
                pos += 4
                if let color = NveSceneRenderer.color(NveColor(Rgba(packed: rgba))) {
                    activeCtx.fill(Path(localBounds), with: .color(color))
                }

            case opFillRect:
                let x = loadF32(pos)
                let y = loadF32(pos + 4)
                let w = loadF32(pos + 8)
                let h = loadF32(pos + 12)
                let rgba = loadU32(pos + 16)
                pos += 20
                if w > 0, h > 0, let color = NveSceneRenderer.color(NveColor(Rgba(packed: rgba))) {
                    activeCtx.fill(Path(CGRect(x: x, y: y, width: w, height: h)), with: .color(color))
                }

            case opFillRoundedRect:
                let x = loadF32(pos)
                let y = loadF32(pos + 4)
                let w = loadF32(pos + 8)
                let h = loadF32(pos + 12)
                let r = max(0, loadF32(pos + 16))
                let rgba = loadU32(pos + 20)
                pos += 24
                if w > 0, h > 0, let color = NveSceneRenderer.color(NveColor(Rgba(packed: rgba))) {
                    let rect = CGRect(x: x, y: y, width: w, height: h)
                    let path = r > 0 ? Path(roundedRect: rect, cornerRadius: r) : Path(rect)
                    activeCtx.fill(path, with: .color(color))
                }

            case opFillCircle:
                let cx = loadF32(pos)
                let cy = loadF32(pos + 4)
                let r = loadF32(pos + 8)
                let rgba = loadU32(pos + 12)
                pos += 16
                if r > 0, let color = NveSceneRenderer.color(NveColor(Rgba(packed: rgba))) {
                    let oval = CGRect(x: cx - r, y: cy - r, width: r * 2, height: r * 2)
                    activeCtx.fill(Path(ellipseIn: oval), with: .color(color))
                }

            case opStrokeLine:
                let x0 = loadF32(pos)
                let y0 = loadF32(pos + 4)
                let x1 = loadF32(pos + 8)
                let y1 = loadF32(pos + 12)
                let width = loadF32(pos + 16)
                let rgba = loadU32(pos + 20)
                pos += 24
                if width > 0, let color = NveSceneRenderer.color(NveColor(Rgba(packed: rgba))) {
                    var path = Path()
                    path.move(to: CGPoint(x: x0, y: y0))
                    path.addLine(to: CGPoint(x: x1, y: y1))
                    activeCtx.stroke(
                        path,
                        with: .color(color),
                        style: StrokeStyle(lineWidth: width, lineCap: .round, lineJoin: .round)
                    )
                }

            case opStrokePolyline:
                let width = loadF32(pos)
                let rgba = loadU32(pos + 4)
                let ptCount = Int(loadU32(pos + 8))
                pos += 12
                if ptCount >= 2, width > 0, let color = NveSceneRenderer.color(NveColor(Rgba(packed: rgba))) {
                    var path = Path()
                    path.move(to: CGPoint(x: loadF32(pos), y: loadF32(pos + 4)))
                    for p in 1 ..< ptCount {
                        let off = pos + p * 8
                        path.addLine(to: CGPoint(x: loadF32(off), y: loadF32(off + 4)))
                    }
                    activeCtx.stroke(
                        path,
                        with: .color(color),
                        style: StrokeStyle(lineWidth: width, lineCap: .round, lineJoin: .round)
                    )
                }
                pos += ptCount * 8

            case opClipRect:
                let x = loadF32(pos)
                let y = loadF32(pos + 4)
                let w = max(0, loadF32(pos + 8))
                let h = max(0, loadF32(pos + 12))
                pos += 16
                if clipStack.count < NveDrawListConstants.maxClipDepth {
                    clipStack.append(activeCtx)
                    activeCtx.clip(to: Path(CGRect(x: x, y: y, width: w, height: h)))
                } else {
                    overflowClipDepth += 1
                }

            case opRestoreClip:
                if overflowClipDepth > 0 {
                    overflowClipDepth -= 1
                } else if let previous = clipStack.popLast() {
                    activeCtx = previous
                }

            default:
                throw NveDrawListError.unknownOpcode(opcode)
            }
        }

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
