import Foundation
import NveHost

/// Outbound NVE event emitted by ``NveControlledTextState``.
public enum NveTextInputAction: Equatable, Sendable {
    case focus
    case blur
    case compositionStart(String)
    case compositionUpdate(String)
    case compositionEnd(String)
    case textChange(String)
    case submit(String)

    public func toEvent(surfaceId: UInt32, nodeId: NodeId, timestampUs: UInt64 = NveEvent.nowUs()) -> NveEvent {
        switch self {
        case .focus:
            return NveEvent(surfaceId: surfaceId, nodeId: nodeId.raw, timestampUs: timestampUs, kind: .focus)
        case .blur:
            return NveEvent(surfaceId: surfaceId, nodeId: nodeId.raw, timestampUs: timestampUs, kind: .blur)
        case let .compositionStart(text):
            return NveEvent(surfaceId: surfaceId, nodeId: nodeId.raw, timestampUs: timestampUs, kind: .compositionStart, payload: .text(text))
        case let .compositionUpdate(text):
            return NveEvent(surfaceId: surfaceId, nodeId: nodeId.raw, timestampUs: timestampUs, kind: .compositionUpdate, payload: .text(text))
        case let .compositionEnd(text):
            return NveEvent(surfaceId: surfaceId, nodeId: nodeId.raw, timestampUs: timestampUs, kind: .compositionEnd, payload: .text(text))
        case let .textChange(text):
            return NveEvent(surfaceId: surfaceId, nodeId: nodeId.raw, timestampUs: timestampUs, kind: .textChange, payload: .text(text))
        case let .submit(text):
            return NveEvent(surfaceId: surfaceId, nodeId: nodeId.raw, timestampUs: timestampUs, kind: .submit, payload: .text(text))
        }
    }
}

/// Platform-agnostic controlled-text + IME composition state machine matching
/// `host-cxx/include/nve/host/controlled_text.hpp` and `NveText.kt`:
/// - Never clobbers active CJK / dead-key marked-text preedit during app re-renders.
/// - Suppresses echoes of in-flight user edits (32-entry FIFO) so async / paired responses never
///   fight the user's caret or newer typing.
/// - Emits balanced `CompositionStart` / `CompositionUpdate` / `CompositionEnd` transitions,
///   suppresses `TextChange` and `TextSubmit` during active composition, and closes active
///   composition cleanly on blur.
public final class NveControlledTextState {
    public static let maxInFlight = 32

    public private(set) var appValue: String = ""
    public private(set) var dispatched: String = ""
    public private(set) var isSynced = false
    public private(set) var isWritingFromHost = false
    public private(set) var isComposing = false
    public private(set) var compositionText: String = ""
    public private(set) var isFocused = false

    private var inFlight: [String] = []
    /// Full committed text before the current IME composition began (used to derive committed delta).
    private var preCompositionText: String = ""
    /// Deferred app value published while IME marked text was active; flushed if composition ends
    /// without a fresh `.textChange` (e.g., cancelled IME).
    private var pendingAppValue: String?

    public init() {}

    /// Runs `body` while host programmatic text writes are ignored by user-edit callbacks.
    public func withHostWrite<R>(_ body: () -> R) -> R {
        let prev = isWritingFromHost
        isWritingFromHost = true
        defer { isWritingFromHost = prev }
        return body()
    }

    /// Reconciles an IME composition that ended natively without firing a text-change callback
    /// (e.g. CJK/dead-key commit or cancel whose final string equals the marked text).
    public func reconcileEndedCompositionIfNeeded(
        currentText: String,
        hasMarkedText: Bool
    ) -> [NveTextInputAction] {
        guard !hasMarkedText && isComposing else { return [] }
        return handleUserEdit(newText: currentText, hasMarkedText: false, markedText: nil)
    }

    /// Decides whether the native control should overwrite its displayed text with the app's
    /// published `value`. Returns `false` while IME marked text is active (recording
    /// `pendingAppValue`) or when `value` is unchanged or echoes an in-flight user edit.
    public func shouldApplyAppValue(
        _ value: String,
        currentShownText: String,
        hasMarkedText: Bool
    ) -> Bool {
        if hasMarkedText {
            pendingAppValue = value
            return false
        }
        // Never allow `isComposing` to deadlock past native `hasMarkedText == false`.
        if isComposing {
            isComposing = false
            compositionText = ""
            preCompositionText = ""
        }
        pendingAppValue = nil
        // Short-circuit unchanged app value before scanning `inFlight`: prevents an unrelated frame
        // carrying the previous `appValue` from evicting an earlier in-flight edit on `A -> B -> A`.
        if isSynced && value == appValue {
            return false
        }
        var shouldWrite = false
        if let echoIndex = inFlight.firstIndex(of: value) {
            inFlight.removeFirst(echoIndex + 1)
        } else if !isSynced || value != currentShownText {
            dispatched = value
            inFlight.removeAll(keepingCapacity: true)
            shouldWrite = true
        }
        appValue = value
        isSynced = true
        return shouldWrite
    }

    /// Processes a native text-change or marked-text update and returns the ordered NVE events
    /// to dispatch (`Composition*`, `TextChange`).
    public func handleUserEdit(
        newText: String,
        hasMarkedText: Bool,
        markedText: String?
    ) -> [NveTextInputAction] {
        guard !isWritingFromHost else { return [] }

        if hasMarkedText {
            let preedit = markedText ?? ""
            if !isComposing {
                isComposing = true
                preCompositionText = dispatched
                compositionText = preedit
                return [.compositionStart(preedit)]
            }
            if preedit != compositionText {
                compositionText = preedit
                return [.compositionUpdate(preedit)]
            }
            return []
        }

        var actions: [NveTextInputAction] = []
        if isComposing {
            isComposing = false
            let committed = committedSegment(from: preCompositionText, to: newText)
            compositionText = ""
            preCompositionText = ""
            actions.append(.compositionEnd(committed))
        }
        if newText != dispatched {
            dispatched = newText
            pendingAppValue = nil
            if inFlight.count >= Self.maxInFlight { inFlight.removeFirst() }
            inFlight.append(newText)
            actions.append(.textChange(newText))
        }
        return actions
    }

    /// Shared edit-reconciliation path used by all AppKit / UIKit coordinators:
    /// 1. Early-returns while `isWritingFromHost` is true (never clamps or touches `inFlight`).
    /// 2. Enforces `maxLength` outside active IME while preserving caret via `writeText`.
    /// 3. Dispatches balanced `Composition*` and `TextChange` actions.
    /// 4. Reconciles synchronous app answers ONLY when `.textChange` was emitted.
    /// 5. Flushes any `pendingAppValue` deferred during IME if composition ended without `.textChange`.
    public func processNativeEdit(
        currentText: String,
        hasMarkedText: Bool,
        markedText: String?,
        maxLength: Int,
        isSynchronous: Bool,
        dispatch: ([NveTextInputAction]) -> Void,
        readAppValue: () -> String?,
        writeText: (String) -> Void
    ) {
        guard !isWritingFromHost else { return }
        var current = currentText
        if !hasMarkedText, maxLength > 0, current.count > maxLength {
            current = String(current.prefix(maxLength))
            withHostWrite { writeText(current) }
        }
        let actions = handleUserEdit(newText: current, hasMarkedText: hasMarkedText, markedText: markedText)
        let emittedTextChange = actions.contains {
            if case .textChange = $0 { return true }
            return false
        }
        if !actions.isEmpty { dispatch(actions) }
        if isSynchronous, emittedTextChange, !hasMarkedText,
           let answer = readAppValue(),
           let writeBack = reconcileSynchronousAnswer(appValue: answer, hasMarkedText: false)
        {
            withHostWrite { writeText(writeBack) }
            return
        }
        if !hasMarkedText, !emittedTextChange, let deferred = pendingAppValue {
            pendingAppValue = nil
            if shouldApplyAppValue(deferred, currentShownText: current, hasMarkedText: false) {
                withHostWrite { writeText(deferred) }
            }
        }
    }

    /// Handles a synchronous local app's post-dispatch value: returns the value to write back
    /// into the native field when the app transformed or rejected the edit, else `nil`.
    public func reconcileSynchronousAnswer(
        appValue answer: String,
        hasMarkedText: Bool
    ) -> String? {
        if hasMarkedText || isComposing {
            pendingAppValue = answer
            return nil
        }
        pendingAppValue = nil
        isSynced = true
        inFlight.removeAll(keepingCapacity: true)
        appValue = answer
        if answer != dispatched {
            dispatched = answer
            return answer
        }
        return nil
    }

    /// Emits `.submit` only when native IME has no marked text. If a stale `isComposing` flag
    /// outlived native `hasMarkedText == false`, closes composition cleanly before submitting.
    public func handleSubmit(currentText: String, hasMarkedText: Bool) -> NveTextInputAction? {
        guard !hasMarkedText else { return nil }
        if isComposing {
            isComposing = false
            compositionText = ""
            preCompositionText = ""
        }
        return .submit(currentText)
    }

    /// Full action sequence for Return/Submit: reconciles any silently ended composition before `.submit`.
    public func handleSubmitActions(currentText: String, hasMarkedText: Bool) -> [NveTextInputAction] {
        guard !hasMarkedText else { return [] }
        var actions = reconcileEndedCompositionIfNeeded(currentText: currentText, hasMarkedText: false)
        actions.append(.submit(currentText))
        return actions
    }

    /// Emits `.focus` / `.blur` only on state transitions; closes active composition on blur.
    public func handleFocusChange(_ focused: Bool) -> [NveTextInputAction] {
        guard focused != isFocused else { return [] }
        isFocused = focused
        if focused {
            return [.focus]
        }
        var actions: [NveTextInputAction] = []
        if isComposing {
            isComposing = false
            compositionText = ""
            preCompositionText = ""
            actions.append(.compositionEnd(""))
        }
        actions.append(.blur)
        return actions
    }

    /// Extracts the newly inserted substring between `before` and `after` (empty if cancelled/erased).
    private func committedSegment(from before: String, to after: String) -> String {
        guard after != before else { return "" }
        let a = Array(before)
        let b = Array(after)
        var prefix = 0
        while prefix < a.count && prefix < b.count && a[prefix] == b[prefix] { prefix += 1 }
        var suffix = 0
        while suffix < (a.count - prefix) && suffix < (b.count - prefix)
            && a[a.count - 1 - suffix] == b[b.count - 1 - suffix]
        {
            suffix += 1
        }
        let end = b.count - suffix
        return end > prefix ? String(b[prefix..<end]) : ""
    }
}
