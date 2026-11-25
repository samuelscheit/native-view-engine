import Foundation
import SwiftUI

/// Fixed `NVE_DRAW1` protocol constants matching `crates/nve-components/src/canvas_ir.rs`.
public enum NveDrawListConstants {
    public static let magic: [UInt8] = Array("NVE_DRAW1".utf8)
    public static let version: UInt16 = 1
    public static let headerSize: Int = 15
    public static let maxOps: Int = 65_536
    public static let maxPolylinePoints: Int = 16_384
    public static let maxClipDepth: Int = 64
    public static let attrPrefix: String = "nve-draw1;b64,"
    public static let attrPrefixBytes: [UInt8] = Array("nve-draw1;b64,".utf8)
}

/// Decoded header of an `NVE_DRAW1` binary draw-list buffer.
public struct NveDrawListHeader: Equatable, Sendable {
    public let version: UInt16
    public let opCount: UInt32

    public init(version: UInt16, opCount: UInt32) {
        self.version = version
        self.opCount = opCount
    }
}

/// Strict validation and decode errors for `NVE_DRAW1` streams.
public enum NveDrawListError: Error, Equatable, Sendable {
    case truncated
    case badMagic
    case unsupportedVersion(UInt16)
    case opCountExceedsLimit(UInt32)
    case polylinePointCountExceedsLimit(UInt32)
    case unknownOpcode(UInt8)
    case nonFiniteFloat
    case trailingBytes(Int)
    case invalidBase64
}

/// Zero-copy `NVE_DRAW1` binary vector draw-list validator & SwiftUI `GraphicsContext` renderer.
///
/// Decodes all 8 opcodes (`Clear`, `FillRect`, `FillRoundedRect`, `FillCircle`, `StrokeLine`,
/// `StrokePolyline`, `ClipRect`, `RestoreClip`) directly from `UnsafeRawBufferPointer` or `Data`
/// with strict bounds, NaN/Inf, vertex-cap, and balanced clip-stack guards.
public enum NveDrawListRenderer {
    public static let opClear: UInt8 = 1
    public static let opFillRect: UInt8 = 2
    public static let opFillRoundedRect: UInt8 = 3
    public static let opFillCircle: UInt8 = 4
    public static let opStrokeLine: UInt8 = 5
    public static let opStrokePolyline: UInt8 = 6
    public static let opClipRect: UInt8 = 7
    public static let opRestoreClip: UInt8 = 8

    /// Strictly validates an `NVE_DRAW1` binary stream zero-copy from `raw`.
    public static func validate(_ raw: UnsafeRawBufferPointer) throws -> NveDrawListHeader {
        let len = raw.count
        guard len >= NveDrawListConstants.headerSize else {
            throw NveDrawListError.truncated
        }
        for i in 0 ..< NveDrawListConstants.magic.count {
            if raw[i] != NveDrawListConstants.magic[i] {
                throw NveDrawListError.badMagic
            }
        }
        let version = UInt16(littleEndian: raw.loadUnaligned(fromByteOffset: 9, as: UInt16.self))
        guard version == NveDrawListConstants.version else {
            throw NveDrawListError.unsupportedVersion(version)
        }
        let opCount = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: 11, as: UInt32.self))
        guard Int(opCount) <= NveDrawListConstants.maxOps else {
            throw NveDrawListError.opCountExceedsLimit(opCount)
        }

        var pos = NveDrawListConstants.headerSize
        guard len - pos >= Int(opCount) else {
            throw NveDrawListError.truncated
        }

        @inline(__always)
        func checkFiniteFloat(at offset: Int) throws {
            let bits = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
            let v = Float(bitPattern: bits)
            if !v.isFinite {
                throw NveDrawListError.nonFiniteFloat
            }
        }

        for _ in 0 ..< Int(opCount) {
            guard pos < len else { throw NveDrawListError.truncated }
            let opcode = raw[pos]
            pos += 1

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
                let ptCount = UInt32(
                    littleEndian: raw.loadUnaligned(fromByteOffset: pos + 8, as: UInt32.self)
                )
                pos += 12
                guard Int(ptCount) <= NveDrawListConstants.maxPolylinePoints else {
                    throw NveDrawListError.polylinePointCountExceedsLimit(ptCount)
                }
                let (needed, overflow) = Int(ptCount).multipliedReportingOverflow(by: 8)
                guard !overflow, len - pos >= needed else {
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
        return NveDrawListHeader(version: version, opCount: opCount)
    }

    /// Validates an `NVE_DRAW1` `Data` payload.
    public static func validate(_ data: Data) throws -> NveDrawListHeader {
        try data.withUnsafeBytes { raw in
            try validate(raw)
        }
    }

    /// Decodes a `nve-draw1;b64,...` or bare base64 string into raw `NVE_DRAW1` bytes.
    public static func decodeAttrString(_ raw: String) throws -> Data {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let b64: String
        if trimmed.hasPrefix(NveDrawListConstants.attrPrefix) {
            b64 = String(trimmed.dropFirst(NveDrawListConstants.attrPrefix.count))
        } else {
            b64 = trimmed
        }
        guard !b64.isEmpty,
              let decoded = Data(base64Encoded: b64, options: .ignoreUnknownCharacters),
              !decoded.isEmpty
        else {
            throw NveDrawListError.invalidBase64
        }
        _ = try validate(decoded)
        return decoded
    }

    /// Zero-copy validates and renders raw `NVE_DRAW1` (or `nve-draw1;b64,...` fallback)
    /// bytes onto a SwiftUI `GraphicsContext` within `bounds`.
    @discardableResult
    public static func draw(
        _ raw: UnsafeRawBufferPointer,
        into context: inout GraphicsContext,
        in bounds: CGRect,
        cornerRadius: CGFloat = 0
    ) throws -> Int {
        if hasAttrPrefix(raw) {
            let b64Slice = UnsafeRawBufferPointer(
                rebasing: raw[NveDrawListConstants.attrPrefixBytes.count ..< raw.count]
            )
            guard let b64Str = String(bytes: b64Slice, encoding: .utf8),
                  let decoded = Data(base64Encoded: b64Str, options: .ignoreUnknownCharacters),
                  !decoded.isEmpty
            else {
                throw NveDrawListError.invalidBase64
            }
            return try draw(decoded, into: &context, in: bounds, cornerRadius: cornerRadius)
        }

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
        var pos = NveDrawListConstants.headerSize

        @inline(__always)
        func loadF32(_ offset: Int) -> CGFloat {
            let bits = UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
            return CGFloat(Float(bitPattern: bits))
        }

        @inline(__always)
        func loadU32(_ offset: Int) -> UInt32 {
            UInt32(littleEndian: raw.loadUnaligned(fromByteOffset: offset, as: UInt32.self))
        }

        for _ in 0 ..< Int(header.opCount) {
            let opcode = raw[pos]
            pos += 1

            switch opcode {
            case opClear:
                let rgba = loadU32(pos)
                pos += 4
                if let color = NveSceneRenderer.color(fromRgba: rgba) {
                    activeCtx.fill(Path(localBounds), with: .color(color))
                }

            case opFillRect:
                let x = loadF32(pos)
                let y = loadF32(pos + 4)
                let w = loadF32(pos + 8)
                let h = loadF32(pos + 12)
                let rgba = loadU32(pos + 16)
                pos += 20
                if w > 0, h > 0, let color = NveSceneRenderer.color(fromRgba: rgba) {
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
                if w > 0, h > 0, let color = NveSceneRenderer.color(fromRgba: rgba) {
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
                if r > 0, let color = NveSceneRenderer.color(fromRgba: rgba) {
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
                if width > 0, let color = NveSceneRenderer.color(fromRgba: rgba) {
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
                if ptCount >= 2, width > 0, let color = NveSceneRenderer.color(fromRgba: rgba) {
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
                }

            case opRestoreClip:
                if let previous = clipStack.popLast() {
                    activeCtx = previous
                }

            default:
                throw NveDrawListError.unknownOpcode(opcode)
            }
        }

        return Int(header.opCount)
    }

    /// Convenience zero-copy overload for `UnsafeBufferPointer<UInt8>` (e.g. `NveSceneReader.textBytes`).
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

    private static func hasAttrPrefix(_ raw: UnsafeRawBufferPointer) -> Bool {
        let prefix = NveDrawListConstants.attrPrefixBytes
        guard raw.count > prefix.count else { return false }
        for i in 0 ..< prefix.count {
            if raw[i] != prefix[i] { return false }
        }
        return true
    }
}
