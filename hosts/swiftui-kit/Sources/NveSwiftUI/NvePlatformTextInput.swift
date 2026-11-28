import CoreGraphics
import CoreText
import Foundation
import NveHost
import SwiftUI

#if os(macOS)
import AppKit
#endif

internal enum NvePlatformTextInputSupport {
    static var usesNativeOverlays: Bool {
        #if os(macOS) || (canImport(UIKit) && !os(watchOS) && !os(tvOS))
        return true
        #else
        return false
        #endif
    }
}

/// Deterministic identity for mounted overlays: toggles of `multiline` or `secure` recreate the
/// backing AppKit / UIKit control cleanly (`NSSecureTextField` vs `NSTextField` vs `NSTextView`).
private struct NveTextInputIdentity: Hashable {
    let nodeId: NodeId
    let multiline: Bool
    let secure: Bool
}

/// Mounts positioned native editable text input overlays for every live `nve.text_input` node.
internal struct NveTextInputOverlayLayer: View {
    let controller: NveSurfaceController
    let inputs: [NveTextInputDescriptor]

    var body: some View {
        ForEach(inputs) { desc in
            let field = NvePlatformTextInput(descriptor: desc, controller: controller)
                .id(NveTextInputIdentity(nodeId: desc.id, multiline: desc.props.multiline, secure: desc.props.secure))
                .frame(width: desc.frame.width, height: desc.frame.height)
            if let clip = desc.visibleClip {
                let localClip = clip.offsetBy(dx: -desc.frame.minX, dy: -desc.frame.minY)
                field
                    .mask(
                        Rectangle()
                            .frame(width: max(0, localClip.width), height: max(0, localClip.height))
                            .position(x: localClip.midX, y: localClip.midY)
                    )
                    .position(x: desc.frame.midX, y: desc.frame.midY)
                    .opacity(desc.opacity)
            } else {
                field
                    .position(x: desc.frame.midX, y: desc.frame.midY)
                    .opacity(desc.opacity)
            }
        }
    }
}

/// Platform-backed editable text input overlay (`NSTextField`/`NSTextView` on macOS,
/// `UITextField`/`UITextView` on iOS / visionOS) with IME guards and controlled-echo suppression.
public struct NvePlatformTextInput: View {
    public let descriptor: NveTextInputDescriptor
    public let controller: NveSurfaceController

    public init(descriptor: NveTextInputDescriptor, controller: NveSurfaceController) {
        self.descriptor = descriptor
        self.controller = controller
    }

    public var body: some View {
        #if os(macOS)
        NveMacTextInputRepresentable(descriptor: descriptor, controller: controller)
        #elseif canImport(UIKit) && !os(watchOS) && !os(tvOS)
        NveUIKitTextInputRepresentable(descriptor: descriptor, controller: controller)
        #else
        NveFallbackTextInputView(descriptor: descriptor, controller: controller)
        #endif
    }
}

// MARK: - macOS (`NSTextField` / `NSSecureTextField` / `NSTextView`)

#if os(macOS)

private func macAlignment(_ align: NveTextAlign) -> NSTextAlignment {
    switch align {
    case .center: .center
    case .right, .end: .right
    case .justify: .justified
    case .left: .left
    case .start: .natural
    }
}

private func macFont(for style: NveTextStyle) -> NSFont {
    if let ct = NveTypographyEngine.shared.font(
        size: style.fontSize, weight: style.fontWeight, italic: style.italic, family: style.fontFamily)
    {
        return ct as NSFont
    }
    return NSFont.systemFont(ofSize: style.fontSize)
}

private func macColor(for c: NveColor?) -> NSColor {
    guard let c else { return .labelColor }
    return NSColor(srgbRed: c.red, green: c.green, blue: c.blue, alpha: c.alpha)
}

private final class NvePlatformNSTextView: NSTextView {
    var onMarkedTextChanged: (() -> Void)?
    var placeholderString: String = "" { didSet { if oldValue != placeholderString { needsDisplay = true } } }
    var placeholderColor: NSColor = .placeholderTextColor

    override func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        super.setMarkedText(string, selectedRange: selectedRange, replacementRange: replacementRange)
        onMarkedTextChanged?()
    }

    override func unmarkText() {
        super.unmarkText()
        onMarkedTextChanged?()
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        if string.isEmpty && !placeholderString.isEmpty {
            let attrs: [NSAttributedString.Key: Any] = [
                .font: font ?? NSFont.systemFont(ofSize: 16),
                .foregroundColor: placeholderColor,
            ]
            let inset = textContainerInset
            let pad = textContainer?.lineFragmentPadding ?? 5
            (placeholderString as NSString).draw(at: NSPoint(x: inset.width + pad, y: inset.height), withAttributes: attrs)
        }
    }
}

private struct NveMacTextInputRepresentable: NSViewRepresentable {
    let descriptor: NveTextInputDescriptor
    let controller: NveSurfaceController

    func makeCoordinator() -> Coordinator {
        Coordinator(descriptor: descriptor, controller: controller)
    }

    func makeNSView(context: Context) -> NSView {
        if descriptor.props.multiline {
            let scroll = NSScrollView(frame: .zero)
            scroll.drawsBackground = false
            scroll.borderType = .noBorder
            scroll.hasVerticalScroller = true
            scroll.autohidesScrollers = true
            let tv = NvePlatformNSTextView(frame: .zero)
            tv.drawsBackground = false
            tv.isRichText = false
            tv.allowsUndo = true
            tv.isVerticallyResizable = true
            tv.isHorizontallyResizable = false
            tv.autoresizingMask = [.width]
            tv.textContainer?.widthTracksTextView = true
            tv.textContainerInset = NSSize(width: 4, height: 4)
            tv.delegate = context.coordinator
            tv.onMarkedTextChanged = { [weak coord = context.coordinator, weak tv] in
                guard let coord, let tv else { return }
                coord.handleTextViewEdit(tv)
            }
            scroll.documentView = tv
            context.coordinator.textView = tv
            applyMultilineProps(to: tv, coordinator: context.coordinator)
            return scroll
        } else {
            let field = descriptor.props.secure ? NSSecureTextField(frame: .zero) : NSTextField(frame: .zero)
            field.isBordered = false
            field.drawsBackground = false
            field.focusRingType = .none
            field.usesSingleLineMode = true
            field.lineBreakMode = .byClipping
            field.cell?.wraps = false
            field.cell?.isScrollable = true
            field.delegate = context.coordinator
            context.coordinator.textField = field
            applySingleLineProps(to: field, coordinator: context.coordinator)
            return field
        }
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.descriptor = descriptor
        if let tv = context.coordinator.textView {
            applyMultilineProps(to: tv, coordinator: context.coordinator)
        } else if let field = context.coordinator.textField {
            applySingleLineProps(to: field, coordinator: context.coordinator)
        }
    }

    private func applySingleLineProps(to field: NSTextField, coordinator: Coordinator) {
        let props = descriptor.props
        field.placeholderString = props.placeholder
        field.isEnabled = !props.disabled
        field.isEditable = !props.readOnly && !props.disabled
        field.isSelectable = !props.disabled
        field.font = macFont(for: descriptor.style)
        field.textColor = macColor(for: descriptor.style.color)
        field.alignment = macAlignment(descriptor.style.align)
        field.setAccessibilityLabel(descriptor.accessibilityLabel)
        field.setAccessibilityHelp(descriptor.accessibilityHint)

        let editor = field.currentEditor() as? NSTextView
        let hasMarked = editor.map { $0.hasMarkedText() || $0.markedRange().length > 0 } ?? false
        if coordinator.state.shouldApplyAppValue(props.value, currentShownText: field.stringValue, hasMarkedText: hasMarked) {
            coordinator.state.withHostWrite {
                let prevSel = editor?.selectedRange()
                field.stringValue = props.value
                if let editor, let sel = prevSel {
                    let utf16Len = (props.value as NSString).length
                    let clampedLoc = min(sel.location, utf16Len)
                    let clampedLen = min(sel.length, utf16Len - clampedLoc)
                    editor.setSelectedRange(NSRange(location: clampedLoc, length: clampedLen))
                }
            }
        }
    }

    private func applyMultilineProps(to tv: NvePlatformNSTextView, coordinator: Coordinator) {
        let props = descriptor.props
        let fg = macColor(for: descriptor.style.color)
        tv.isEditable = !props.readOnly && !props.disabled
        tv.isSelectable = !props.disabled
        tv.font = macFont(for: descriptor.style)
        tv.textColor = fg
        tv.insertionPointColor = fg
        tv.alignment = macAlignment(descriptor.style.align)
        tv.placeholderString = props.placeholder
        tv.placeholderColor = fg.withAlphaComponent(0.45)
        tv.setAccessibilityLabel(descriptor.accessibilityLabel)
        tv.setAccessibilityHelp(descriptor.accessibilityHint)

        let hasMarked = tv.hasMarkedText() || tv.markedRange().length > 0
        if coordinator.state.shouldApplyAppValue(props.value, currentShownText: tv.string, hasMarkedText: hasMarked) {
            coordinator.state.withHostWrite {
                let prevSel = tv.selectedRange()
                tv.string = props.value
                let utf16Len = (props.value as NSString).length
                let clampedLoc = min(prevSel.location, utf16Len)
                let clampedLen = min(prevSel.length, utf16Len - clampedLoc)
                tv.setSelectedRange(NSRange(location: clampedLoc, length: clampedLen))
            }
        }
    }

    final class Coordinator: NSObject, NSTextFieldDelegate, NSTextViewDelegate {
        var descriptor: NveTextInputDescriptor
        let controller: NveSurfaceController
        let state = NveControlledTextState()
        weak var textField: NSTextField?
        weak var textView: NvePlatformNSTextView?

        init(descriptor: NveTextInputDescriptor, controller: NveSurfaceController) {
            self.descriptor = descriptor
            self.controller = controller
        }

        private func emit(_ actions: [NveTextInputAction]) {
            guard !actions.isEmpty else { return }
            let sid = controller.surfaceId
            let nid = descriptor.id
            let ts = NveEvent.nowUs()
            controller.dispatchEvents(actions.map { $0.toEvent(surfaceId: sid, nodeId: nid, timestampUs: ts) })
        }

        private func markedText(in tv: NSTextView?) -> (hasMarked: Bool, text: String?) {
            guard let tv else { return (false, nil) }
            let range = tv.markedRange()
            let hasMarked = tv.hasMarkedText() || range.length > 0
            guard hasMarked else { return (false, nil) }
            let ns = tv.string as NSString
            guard range.location != NSNotFound, range.location + range.length <= ns.length else {
                return (true, "")
            }
            return (true, ns.substring(with: range))
        }

        func controlTextDidBeginEditing(_ obj: Notification) { emit(state.handleFocusChange(true)) }
        func controlTextDidEndEditing(_ obj: Notification) { emit(state.handleFocusChange(false)) }

        func controlTextDidChange(_ obj: Notification) {
            guard let field = obj.object as? NSTextField else { return }
            let editor = (obj.userInfo?["NSFieldEditor"] as? NSTextView) ?? (field.currentEditor() as? NSTextView)
            let marked = markedText(in: editor)
            var current = field.stringValue
            if !marked.hasMarked, descriptor.props.maxLength > 0, current.count > descriptor.props.maxLength {
                current = String(current.prefix(descriptor.props.maxLength))
                state.withHostWrite { field.stringValue = current }
            }
            let actions = state.handleUserEdit(newText: current, hasMarkedText: marked.hasMarked, markedText: marked.text)
            emit(actions)
            if controller.dispatchesSynchronously, !marked.hasMarked,
               let answer = controller.store.textInputValue(for: descriptor.id),
               let writeBack = state.reconcileSynchronousAnswer(appValue: answer, hasMarkedText: false)
            {
                state.withHostWrite { field.stringValue = writeBack }
            }
        }

        func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
            guard commandSelector == #selector(NSResponder.insertNewline(_:)),
                  let field = control as? NSTextField
            else { return false }
            let marked = markedText(in: textView)
            guard let action = state.handleSubmit(currentText: field.stringValue, hasMarkedText: marked.hasMarked) else {
                return false
            }
            emit([action])
            return true
        }

        func textDidBeginEditing(_ notification: Notification) { emit(state.handleFocusChange(true)) }
        func textDidEndEditing(_ notification: Notification) { emit(state.handleFocusChange(false)) }

        func textDidChange(_ notification: Notification) {
            guard let tv = notification.object as? NSTextView else { return }
            handleTextViewEdit(tv)
        }

        func handleTextViewEdit(_ tv: NSTextView) {
            let marked = markedText(in: tv)
            var current = tv.string
            if !marked.hasMarked, descriptor.props.maxLength > 0, current.count > descriptor.props.maxLength {
                current = String(current.prefix(descriptor.props.maxLength))
                state.withHostWrite { tv.string = current }
            }
            let actions = state.handleUserEdit(newText: current, hasMarkedText: marked.hasMarked, markedText: marked.text)
            emit(actions)
            if controller.dispatchesSynchronously, !marked.hasMarked,
               let answer = controller.store.textInputValue(for: descriptor.id),
               let writeBack = state.reconcileSynchronousAnswer(appValue: answer, hasMarkedText: false)
            {
                state.withHostWrite { tv.string = writeBack }
            }
        }
    }
}

#endif
