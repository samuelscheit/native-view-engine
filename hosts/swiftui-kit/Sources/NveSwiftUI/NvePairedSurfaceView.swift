import CNveAbi
import NveHost
import Foundation
import SwiftUI

/// Thin-client dev surface: pairs a ``NveSurfaceController``'s runtime with a remote `nve dev`
/// server (`pair_connect`: authenticated wire session, automatic reconnect + keyframe recovery)
/// and renders the replicated scene natively.
public struct NvePairedSurfaceView: View {
    @ObservedObject public var controller: NveSurfaceController
    @State private var uriInput: String
    @State private var tokenInput: String
    @State private var errorMessage: String?
    @State private var metrics = NveRuntimeMetrics()

    public init(
        controller: NveSurfaceController,
        initialUri: String = "nve://pair?host=127.0.0.1&ws=8092&token=dev",
        initialToken: String = ""
    ) {
        self.controller = controller
        _uriInput = State(initialValue: initialUri)
        _tokenInput = State(initialValue: initialToken)
    }

    public var body: some View {
        VStack(spacing: 10) {
            pairingBar
            telemetry
            if let errorMessage {
                Text(errorMessage)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            ZStack {
                NveSceneCanvasView(controller: controller)
                if controller.isPaired && metrics.frame_id == 0 {
                    Text("Waiting for first keyframe…").font(.subheadline).foregroundStyle(.secondary)
                } else if !controller.isPaired {
                    Text("Not paired").font(.subheadline).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(12)
        .onReceive(controller.store.$frameId) { _ in refreshMetrics() }
    }

    private var pairingBar: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                Circle()
                    .fill(controller.isPaired ? Color.green : Color.secondary)
                    .frame(width: 9, height: 9)
                Text(controller.isPaired ? "Paired" : "Disconnected")
                    .font(.caption.weight(.semibold))
                    .lineLimit(1)
                Spacer()
                Button(controller.isPaired ? "Disconnect" : "Connect", action: toggle)
                    .buttonStyle(.borderedProminent)
                    .nveAdaptiveSmallControl()
            }
            HStack(spacing: 8) {
                TextField("nve://pair?host=127.0.0.1&ws=8092&token=...", text: $uriInput)
                    .nveAdaptiveTextFieldStyle()
                    .font(.system(.caption, design: .monospaced))
                    .autocorrectionDisabled(true)
                TextField("Token (optional)", text: $tokenInput)
                    .nveAdaptiveTextFieldStyle()
                    .font(.system(.caption, design: .monospaced))
                    .frame(maxWidth: 150)
                    .autocorrectionDisabled(true)
            }
        }
    }

    private var telemetry: some View {
        HStack(spacing: 12) {
            metric("Frame", "\(metrics.frame_id)")
            metric("Nodes", "\(metrics.live_nodes)")
            metric("RX", Self.bytes(metrics.wire_bytes_rx))
            metric("TX", Self.bytes(metrics.wire_bytes_tx))
            metric("Layout", "\(metrics.layout_us)µs")
            Spacer()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.secondary.opacity(0.12)))
    }

    private func metric(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(label).font(.caption2).foregroundStyle(.secondary)
            Text(value).font(.caption.monospacedDigit().weight(.semibold))
        }
    }

    private func toggle() {
        errorMessage = nil
        do {
            if controller.isPaired {
                try controller.unpair()
            } else {
                try controller.pair(uri: uriInput.trimmingCharacters(in: .whitespacesAndNewlines),
                                    token: tokenInput.trimmingCharacters(in: .whitespacesAndNewlines))
            }
        } catch let NveRuntimeError.status(_, message) {
            errorMessage = message.isEmpty ? "Pairing failed" : message
        } catch {
            errorMessage = "\(error)"
        }
        refreshMetrics()
    }

    private func refreshMetrics() {
        if let m = try? controller.session.metrics() { metrics = m }
    }

    static func bytes(_ n: UInt64) -> String {
        n >= 1 << 20 ? String(format: "%.1fMB", Double(n) / 1_048_576) : n >= 1 << 10 ? String(format: "%.1fKB", Double(n) / 1024) : "\(n)B"
    }
}
