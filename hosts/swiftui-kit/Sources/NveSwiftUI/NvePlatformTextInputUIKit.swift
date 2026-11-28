import CoreGraphics
import CoreText
import Foundation
import NveHost
import SwiftUI

#if canImport(UIKit) && !os(watchOS) && !os(tvOS)
import UIKit

private func uiAlignment(_ align: NveTextAlign) -> NSTextAlignment {
    switch align {
    case .center: .center
    case .right, .end: .right
    case .justify: .justified
    case .left: .left
    case .start: .natural
    }
}

private func uiFont(for style: NveTextStyle) -> UIFont {
    if let ct = NveTypographyEngine.shared.font(
        size: style.fontSize, weight: style.fontWeight, italic: style.italic, family: style.fontFamily)
    {
        return ct as UIFont
    }
    return UIFont.systemFont(ofSize: style.fontSize)
}

private func uiColor(for c: NveColor?) -> UIColor {
    guard let c else { return .label }
    return UIColor(red: c.red, green: c.green, blue: c.blue, alpha: c.alpha)
}

private func uiKeyboardType(for kind: NveInputKind) -> UIKeyboardType {
    switch kind {
    case .email: .emailAddress
    case .tel: .phonePad
    case .url: .URL
    case .number: .decimalPad
    case .search: .webSearch
    default: .default
    }
}

/// UIKit host container enforcing true visual AND hit-test clipping (`clipsToBounds`) to
/// scroll/overflow viewports while positioning the full-size input control at its scrolled offset.
internal final class NveUIKitClippedInputHostView: UIView {
    let contentView: UIView
    var contentRect: CGRect = .zero {
        didSet {
            if contentView.frame != contentRect { contentView.frame = contentRect }
        }
    }

    init(contentView: UIView) {
        self.contentView = contentView
        super.init(frame: .zero)
        clipsToBounds = true
        backgroundColor = .clear
        addSubview(contentView)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layoutSubviews() {
        super.layoutSubviews()
        if contentView.frame != contentRect { contentView.frame = contentRect }
    }
}

internal struct NveUIKitTextInputRepresentable: UIViewRepresentable {
    let descriptor: NveTextInputDescriptor
    let controller: NveSurfaceController

    func makeCoordinator() -> Coordinator {
        Coordinator(descriptor: descriptor, controller: controller)
    }

    func makeUIView(context: Context) -> NveUIKitClippedInputHostView {
        let content: UIView
        if descriptor.props.multiline {
            let tv = UITextView(frame: .zero)
            tv.backgroundColor = .clear
            tv.delegate = context.coordinator
            let placeholder = UILabel()
            placeholder.numberOfLines = 0
            placeholder.isUserInteractionEnabled = false
            placeholder.translatesAutoresizingMaskIntoConstraints = false
            tv.addSubview(placeholder)
            NSLayoutConstraint.activate([
                placeholder.topAnchor.constraint(equalTo: tv.topAnchor, constant: 8),
                placeholder.leadingAnchor.constraint(equalTo: tv.leadingAnchor, constant: 5),
                placeholder.trailingAnchor.constraint(lessThanOrEqualTo: tv.frameLayoutGuide.trailingAnchor, constant: -5),
            ])
            context.coordinator.textView = tv
            context.coordinator.placeholderLabel = placeholder
            applyMultilineProps(to: tv, coordinator: context.coordinator)
            content = tv
        } else {
            let field = UITextField(frame: .zero)
            field.backgroundColor = .clear
            field.borderStyle = .none
            field.delegate = context.coordinator
            field.addTarget(
                context.coordinator,
                action: #selector(Coordinator.onTextFieldEditingChanged(_:)),
                for: .editingChanged
            )
            context.coordinator.textField = field
            applySingleLineProps(to: field, coordinator: context.coordinator)
            content = field
        }
        let host = NveUIKitClippedInputHostView(contentView: content)
        updateHostGeometry(host)
        return host
    }

    func updateUIView(_ uiView: NveUIKitClippedInputHostView, context: Context) {
        context.coordinator.descriptor = descriptor
        updateHostGeometry(uiView)
        if let tv = context.coordinator.textView {
            applyMultilineProps(to: tv, coordinator: context.coordinator)
        } else if let field = context.coordinator.textField {
            applySingleLineProps(to: field, coordinator: context.coordinator)
        }
    }

    static func dismantleUIView(_ uiView: NveUIKitClippedInputHostView, coordinator: Coordinator) {
        coordinator.teardown()
    }

    private func updateHostGeometry(_ host: NveUIKitClippedInputHostView) {
        let outer = descriptor.visibleClip.map { descriptor.frame.intersection($0) } ?? descriptor.frame
        let offset = CGPoint(x: descriptor.frame.minX - outer.minX, y: descriptor.frame.minY - outer.minY)
        host.contentRect = CGRect(origin: offset, size: descriptor.frame.size)
    }

    private func applySingleLineProps(to field: UITextField, coordinator: Coordinator) {
        let props = descriptor.props
        field.placeholder = props.placeholder
        field.isSecureTextEntry = props.secure
        field.isEnabled = !props.disabled
        field.font = uiFont(for: descriptor.style)
        field.textColor = uiColor(for: descriptor.style.color)
        field.textAlignment = uiAlignment(descriptor.style.align)
        field.keyboardType = uiKeyboardType(for: props.inputKind)
        field.returnKeyType = props.inputKind == .search ? .search : .done
        field.accessibilityLabel = descriptor.accessibilityLabel
        field.accessibilityHint = descriptor.accessibilityHint

        let hasMarked = field.markedTextRange != nil
        coordinator.reconcileEndedComposition(currentText: field.text ?? "", hasMarkedText: hasMarked)
        if coordinator.state.shouldApplyAppValue(props.value, currentShownText: field.text ?? "", hasMarkedText: hasMarked) {
            coordinator.state.withHostWrite {
                Coordinator.writePreservingSelection(props.value, to: field)
            }
        }
    }

    private func applyMultilineProps(to tv: UITextView, coordinator: Coordinator) {
        let props = descriptor.props
        let fg = uiColor(for: descriptor.style.color)
        tv.isEditable = !props.readOnly && !props.disabled
        tv.isSelectable = !props.disabled
        tv.font = uiFont(for: descriptor.style)
        tv.textColor = fg
        tv.textAlignment = uiAlignment(descriptor.style.align)
        tv.keyboardType = uiKeyboardType(for: props.inputKind)
        tv.accessibilityLabel = descriptor.accessibilityLabel
        tv.accessibilityHint = descriptor.accessibilityHint

        if let ph = coordinator.placeholderLabel {
            ph.text = props.placeholder
            ph.font = tv.font
            ph.textColor = fg.withAlphaComponent(0.45)
            ph.isHidden = !tv.text.isEmpty || props.placeholder.isEmpty
        }

        let hasMarked = tv.markedTextRange != nil
        coordinator.reconcileEndedComposition(currentText: tv.text ?? "", hasMarkedText: hasMarked)
        if coordinator.state.shouldApplyAppValue(props.value, currentShownText: tv.text ?? "", hasMarkedText: hasMarked) {
            coordinator.state.withHostWrite {
                Coordinator.writePreservingSelection(props.value, to: tv)
            }
            coordinator.placeholderLabel?.isHidden = !props.value.isEmpty || props.placeholder.isEmpty
        }
    }

    final class Coordinator: NSObject, UITextFieldDelegate, UITextViewDelegate {
        var descriptor: NveTextInputDescriptor
        let controller: NveSurfaceController
        let state = NveControlledTextState()
        weak var textField: UITextField?
        weak var textView: UITextView?
        weak var placeholderLabel: UILabel?

        init(descriptor: NveTextInputDescriptor, controller: NveSurfaceController) {
            self.descriptor = descriptor
            self.controller = controller
        }

        func teardown() {
            textField?.removeTarget(self, action: #selector(onTextFieldEditingChanged(_:)), for: .editingChanged)
            textField?.delegate = nil
            textView?.delegate = nil
            if state.isFocused {
                emit(state.handleFocusChange(false))
            } else if state.isComposing {
                emit(state.reconcileEndedCompositionIfNeeded(currentText: state.dispatched, hasMarkedText: false))
            }
        }

        func reconcileEndedComposition(currentText: String, hasMarkedText: Bool) {
            emit(state.reconcileEndedCompositionIfNeeded(currentText: currentText, hasMarkedText: hasMarkedText))
        }

        static func writePreservingSelection(_ text: String, to field: UITextField) {
            let prev = field.selectedTextRange
            field.text = text
            if let prev {
                let utf16Len = (text as NSString).length
                let startOff = min(field.offset(from: field.beginningOfDocument, to: prev.start), utf16Len)
                let endOff = min(field.offset(from: field.beginningOfDocument, to: prev.end), utf16Len)
                if let s = field.position(from: field.beginningOfDocument, offset: startOff),
                   let e = field.position(from: field.beginningOfDocument, offset: max(startOff, endOff))
                {
                    field.selectedTextRange = field.textRange(from: s, to: e)
                }
            }
        }

        static func writePreservingSelection(_ text: String, to tv: UITextView) {
            let prev = tv.selectedRange
            tv.text = text
            let utf16Len = (text as NSString).length
            let clampedLoc = min(prev.location, utf16Len)
            tv.selectedRange = NSRange(location: clampedLoc, length: min(prev.length, utf16Len - clampedLoc))
        }

        private func emit(_ actions: [NveTextInputAction]) {
            guard !actions.isEmpty else { return }
            let sid = controller.surfaceId
            let nid = descriptor.id
            let ts = NveEvent.nowUs()
            controller.dispatchEvents(actions.map { $0.toEvent(surfaceId: sid, nodeId: nid, timestampUs: ts) })
        }

        func textFieldShouldBeginEditing(_ textField: UITextField) -> Bool {
            !descriptor.props.readOnly && !descriptor.props.disabled
        }

        func textFieldDidBeginEditing(_ textField: UITextField) { emit(state.handleFocusChange(true)) }
        func textFieldDidEndEditing(_ textField: UITextField) { emit(state.handleFocusChange(false)) }

        func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
            !descriptor.props.readOnly && !descriptor.props.disabled
        }

        @objc func onTextFieldEditingChanged(_ field: UITextField) {
            let markedRange = field.markedTextRange
            let hasMarked = markedRange != nil
            let markedText = markedRange.flatMap { field.text(in: $0) }
            state.processNativeEdit(
                currentText: field.text ?? "",
                hasMarkedText: hasMarked,
                markedText: markedText,
                maxLength: descriptor.props.maxLength,
                isSynchronous: controller.dispatchesSynchronously,
                dispatch: emit,
                readAppValue: { controller.store.textInputValue(for: descriptor.id) },
                writeText: { Self.writePreservingSelection($0, to: field) }
            )
        }

        func textFieldShouldReturn(_ textField: UITextField) -> Bool {
            guard !descriptor.props.readOnly, !descriptor.props.disabled else { return false }
            let actions = state.handleSubmitActions(
                currentText: textField.text ?? "", hasMarkedText: textField.markedTextRange != nil
            )
            guard !actions.isEmpty else { return false }
            emit(actions)
            return true
        }

        func textViewDidBeginEditing(_ textView: UITextView) { emit(state.handleFocusChange(true)) }
        func textViewDidEndEditing(_ textView: UITextView) { emit(state.handleFocusChange(false)) }

        func textView(_ textView: UITextView, shouldChangeTextIn range: NSRange, replacementText text: String) -> Bool {
            !descriptor.props.readOnly && !descriptor.props.disabled
        }

        func textViewDidChangeSelection(_ textView: UITextView) {
            guard state.isComposing, textView.markedTextRange == nil else { return }
            textViewDidChange(textView)
        }

        func textViewDidChange(_ textView: UITextView) {
            let markedRange = textView.markedTextRange
            let hasMarked = markedRange != nil
            let markedText = markedRange.flatMap { textView.text(in: $0) }
            placeholderLabel?.isHidden = !(textView.text ?? "").isEmpty || descriptor.props.placeholder.isEmpty
            state.processNativeEdit(
                currentText: textView.text ?? "",
                hasMarkedText: hasMarked,
                markedText: markedText,
                maxLength: descriptor.props.maxLength,
                isSynchronous: controller.dispatchesSynchronously,
                dispatch: emit,
                readAppValue: { controller.store.textInputValue(for: descriptor.id) },
                writeText: { [weak self] next in
                    Self.writePreservingSelection(next, to: textView)
                    self?.placeholderLabel?.isHidden = !next.isEmpty || (self?.descriptor.props.placeholder.isEmpty ?? true)
                }
            )
        }
    }
}

#elseif !os(macOS)

internal struct NveFallbackTextInputView: View {
    let descriptor: NveTextInputDescriptor
    let controller: NveSurfaceController
    @State private var text: String = ""

    var body: some View {
        Group {
            if descriptor.props.secure {
                SecureField(descriptor.props.placeholder, text: $text)
            } else {
                TextField(descriptor.props.placeholder, text: $text)
            }
        }
        .disabled(descriptor.props.disabled || descriptor.props.readOnly)
        .onAppear { text = descriptor.props.value }
        .onChange(of: descriptor.props.value) { text = $0 }
        .onChange(of: text) { next in
            guard next != descriptor.props.value else { return }
            controller.dispatchEvent(NveEvent(
                surfaceId: controller.surfaceId, nodeId: descriptor.id.raw,
                timestampUs: NveEvent.nowUs(), kind: .textChange, payload: .text(next)))
        }
        .onSubmit {
            controller.dispatchEvent(NveEvent(
                surfaceId: controller.surfaceId, nodeId: descriptor.id.raw,
                timestampUs: NveEvent.nowUs(), kind: .submit, payload: .text(text)))
        }
    }
}

#endif
