import CNveAbi
import NveHost

/// C ABI bridge between pure-Swift `EventBatchBuilder` / `EventBatchReader` and `CNveAbi.NveEventBatch`.
public extension EventBatchBuilder {
    /// Borrows a zero-copy, zero-allocation `CNveAbi.NveEventBatch` pointing directly at this
    /// builder's 8-byte-aligned 64-byte record slab and byte arena (valid only inside `body`).
    func withRawBatch<R>(_ body: (CNveAbi.NveEventBatch) throws -> R) rethrows -> R {
        guard !isEmpty else {
            return try body(CNveAbi.NveEventBatch(events: nil, bytes: nil, event_count: 0, bytes_len: 0))
        }
        precondition(
            MemoryLayout<CNveAbi.NveEvent>.size == EventWire.recordSize
                && MemoryLayout<CNveAbi.NveEvent>.stride == EventWire.recordSize
                && MemoryLayout<CNveAbi.NveEvent>.alignment <= MemoryLayout<UInt64>.alignment,
            "CNveAbi.NveEvent layout invariant violated"
        )
        return try withRecordWords { wordsBuf in
            try wordsBuf.withMemoryRebound(to: CNveAbi.NveEvent.self) { evBuf in
                try withArenaBytes { arenaRaw in
                    let bytesPtr = arenaRaw.isEmpty ? nil : arenaRaw.bindMemory(to: UInt8.self).baseAddress
                    let batch = CNveAbi.NveEventBatch(
                        events: evBuf.baseAddress,
                        bytes: bytesPtr,
                        event_count: UInt32(count),
                        bytes_len: UInt32(bytes.count)
                    )
                    return try body(batch)
                }
            }
        }
    }

    /// Validates a borrowed `CNveAbi.NveEventBatch` (e.g. from `drain_emitted_events`) and copies
    /// its records and byte arena into an owned `EventBatchBuilder` safe to keep across FFI calls.
    static func copy(from batch: CNveAbi.NveEventBatch) throws(EventValidationError) -> EventBatchBuilder {
        copy(from: try EventBatchReader.validate(batch: batch))
    }
}

public extension EventBatchReader {
    /// Single-pass zero-copy, zero-allocation ingress validation of a C ABI `CNveAbi.NveEventBatch`
    /// (mirrors Rust `ValidatedEvents::try_from_raw`).
    static func validate(batch: CNveAbi.NveEventBatch) throws(EventValidationError) -> EventBatchReader {
        let evCount64 = UInt64(batch.event_count)
        let bytesLen64 = UInt64(batch.bytes_len)

        let arenaBuf: UnsafeRawBufferPointer
        if bytesLen64 == 0 {
            arenaBuf = UnsafeRawBufferPointer(start: nil, count: 0)
        } else {
            guard let ptr = batch.bytes else { throw .nullBytesPointer }
            guard let n = TableReader.fitsAddressSpace(bytesLen64, stride: 1) else {
                throw .bytesLengthOverflow
            }
            arenaBuf = UnsafeRawBufferPointer(start: UnsafeRawPointer(ptr), count: n)
        }

        let recBuf: UnsafeRawBufferPointer
        let evCount: Int
        if evCount64 == 0 {
            recBuf = UnsafeRawBufferPointer(start: nil, count: 0)
            evCount = 0
        } else {
            guard let ptr = batch.events else { throw .nullEventsPointer }
            guard let n = TableReader.fitsAddressSpace(evCount64, stride: EventWire.recordSize) else {
                throw .eventsLengthOverflow
            }
            recBuf = UnsafeRawBufferPointer(start: UnsafeRawPointer(ptr), count: n * EventWire.recordSize)
            evCount = n
        }

        return try validate(records: recBuf, count: evCount, arena: arenaBuf)
    }
}
