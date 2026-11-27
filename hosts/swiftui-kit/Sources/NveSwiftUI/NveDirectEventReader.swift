#if canImport(CNveAbi)
import CNveAbi
#endif
import Foundation

// MARK: - Zero-Copy Direct C-ABI Event Batch Reader

public extension NveDirectEventWriter {
    /// Strictly decodes a direct C-ABI `NveEventBatch` via `NveEventRingCodec`'s single-source
    /// strict validator (bounds, framing flags, UTF-8, table/event IDs).
    static func decodeBatchStrict(_ batch: NveEventBatch) throws -> [NveEvent] {
        let count = Int(batch.event_count)
        let arenaLen = Int(batch.bytes_len)
        if arenaLen > 0 && batch.bytes == nil {
            throw NveEventRingError.invalidBounds
        }
        if count == 0 {
            return []
        }
        guard let evPtr = batch.events else {
            throw NveEventRingError.invalidBounds
        }
        let recBytes = count * NveEventRingCodec.eventRecordBytes
        let records = UnsafeRawBufferPointer(start: UnsafeRawPointer(evPtr), count: recBytes)
        let arena = UnsafeRawBufferPointer(start: batch.bytes.map { UnsafeRawPointer($0) }, count: arenaLen)
        return try NveEventRingCodec.decodeRawBufferStrict(records: records, count: count, arena: arena)
    }

    static func decodeBatch(_ batch: NveEventBatch) -> [NveEvent] {
        (try? decodeBatchStrict(batch)) ?? []
    }
}
