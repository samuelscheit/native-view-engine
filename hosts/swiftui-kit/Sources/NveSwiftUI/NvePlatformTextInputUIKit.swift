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

internal struct NveUIKitTextInputRepresentable: UIViewRepresentable {
    let descriptor: NveTextInputDescriptor
    let controller: NveSurfaceController

    func makeCoordinator() -> Coordinator {
        Coordinator(descriptor: descriptor, controller: controller)
    }

    func makeUIView(context: Context) -> UIView {
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
                placeholder.trailingAnchor.constraint(lessThanOrEqualTo: tv.trailingAnchor, constant: -5),
            ])
            context.coordinator.textView = tv
            context.coordinator.placeholderLabel = placeholder
            applyMultilineProps(to: tv, coordinator: context.coordinator)
            return tv
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
            return field
        }
    }

    func updateUIView(_ uiView: UIView, context: Context) {
        context.coordinator.descriptor = descriptor
        if let tv = context.coordinator.textView {
            applyMultilineProps(to: tv, coordinator: context.coordinator)
        } else if let field = context.coordinator.textField {
            applySingleLineProps(to: field, coordinator: context.coordinator)
        }
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
        if coordinator.state.shouldApplyAppValue(props.value, currentShownText: field.text ?? "", hasMarkedText: hasMarked) {
            coordinator.state.withHostWrite {
                let prev = field.selectedTextRange
                field.text = props.value
                if let prev {
                    let utf16Len = (props.value as NSString).length
                    let offset = min(field.offset(from: field.beginningOfDocument, to: prev.start), utf16Len)
                    if let pos = field.position(from: field.beginningOfDocument, offset: offset) {
                        field.selectedTextRange = field.textRange(from: pos, to: pos)
                    }
                }
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
        if coordinator.state.shouldApplyAppValue(props.value, currentShownText: tv.text ?? "", hasMarkedText: hasMarked) {
            coordinator.state.withHostWrite {
                let prev = tv.selectedRange
                tv.text = props.value
                let utf16Len = (props.value as NSString).length
                let clampedLoc = min(prev.location, utf16Len)
                tv.selectedRange = NSRange(location: clampedLoc, length: min(prev.length, utf16Len - clampedLoc))
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

        private func emit(_ actions: [NveTextInputAction]) {
            guard !actions.isEmpty else { return }
            let sid = controller.surfaceId
            let nid = descriptor.id
            let ts = NveEvent.nowUs()
            controller.dispatchEvents(actions.map { $0.toEvent(surfaceId: sid, nodeId: nid, timestampUs: ts) })
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
            var current = field.text ?? ""
            if !hasMarked, descriptor.props.maxLength > 0, current.count > descriptor.props.maxLength {
                current = String(current.prefix(descriptor.props.maxLength))
                state.withHostWrite { field.text = current }
            }
            let actions = state.handleUserEdit(newText: current, hasMarkedText: hasMarked, markedText: markedText)
            emit(actions)
            if controller.dispatchesSynchronously, !hasMarked,
               let answer = controller.store.textInputValue(for: descriptor.id),
               let writeBack = state.reconcileSynchronousAnswer(appValue: answer, hasMarkedText: false)
            {
                state.withHostWrite { field.text = writeBack }
            }
        }

        func textFieldShouldReturn(_ textField: UITextField) -> Bool {
            guard let action = state.handleSubmit(
                currentText: textField.text ?? "", hasMarkedText: textField.markedTextRange != nil)
            else { return false }
            emit([action])
            return true
        }

        func textViewDidBeginEditing(_ textView: UITextView) { emit(state.handleFocusChange(true)) }
        func textViewDidEndEditing(_ textView: UITextView) { emit(state.handleFocusChange(false)) }

        func textView(_ textView: UITextView, shouldChangeTextIn range: NSRange, replacementText text: String) -> Bool {
            !descriptor.props.readOnly && !descriptor.props.disabled
        }

        func textViewDidChange(_ textView: UITextView) {
            let markedRange = textView.markedTextRange
            let hasMarked = markedRange != nil
            let markedText = markedRange.flatMap { textView.text(in: $0) }
            var current = textView.text ?? ""
            if !hasMarked, descriptor.props.maxLength > 0, current.count > descriptor.props.maxLength {
                current = String(current.prefix(descriptor.props.maxLength))
                state.withHostWrite { textView.text = current }
            }
            placeholderLabel?.isHidden = !current.isEmpty || descriptor.props.placeholder.isEmpty
            let actions = state.handleUserEdit(newText: current, hasMarkedText: hasMarked, markedText: markedText)
            emit(actions)
            if controller.dispatchesSynchronously, !hasMarked,
               let answer = controller.store.textInputValue(for: descriptor.id),
               let writeBack = state.reconcileSynchronousAnswer(appValue: answer, hasMarkedText: false)
            {
                state.withHostWrite { textView.text = writeBack }
                placeholderLabel?.isHidden = !writeBack.isEmpty || descriptor.props.placeholder.isEmpty
            }
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
