import Foundation
import SwiftUI

/// Live zero-cdylib SwiftUI thin-client surface paired over `NVT1`/`NVD1` binary WebSockets.
///
/// Renders a compact pairing control bar (URI/token inputs + Connect/Disconnect button),
/// a live telemetry pill (RTT, FPS, RX throughput, compression), and either a caller-supplied
/// paired frame view or a built-in interactive `CommandBuffer` display forwarding tap events
/// back over `InputEvents = 11`.
public struct NvePairedSurfaceView<Content: View>: View {
    @ObservedObject public var client: NvePairClient
    @State private var uriInput: String
    @State private var tokenInput: String
    @State private var validationError: String?

    private let surfaceId: UInt32
    private let frameBuilder: ((UInt32, Data) -> Content)?

    public init(
        client: NvePairClient,
        initialUri: String = "nve://pair?host=127.0.0.1&ws=8092&token=dev",
        initialToken: String = "",
        surfaceId: UInt32 = 1,
        @ViewBuilder content: @escaping (UInt32, Data) -> Content
    ) {
        self.client = client
        self._uriInput = State(initialValue: initialUri)
        self._tokenInput = State(initialValue: initialToken)
        self.surfaceId = surfaceId
        self.frameBuilder = content
    }

    public var body: some View {
        VStack(spacing: 10) {
            pairingBar
            telemetryPill
            if let validationError {
                Text(validationError)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            surfaceBody
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .padding(12)
    }

    private var pairingBar: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 9, height: 9)
                Text(client.state.label)
                    .font(.caption.weight(.semibold))
                    .lineLimit(1)
                Spacer()
                if client.state.isPaired {
                    Button("Keyframe") {
                        client.requestKeyframe()
                    }
                    .buttonStyle(.bordered)
                    .nveAdaptiveSmallControl()
                }
                Button(isActiveConnection ? "Disconnect" : "Connect") {
                    toggleConnection()
                }
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

    private var telemetryPill: some View {
        HStack(spacing: 12) {
            metricLabel("RTT", value: formattedRtt)
            metricLabel("FPS", value: String(format: "%.0f", client.telemetry.fps))
            metricLabel("RX", value: formattedThroughput(client.telemetry.bytesPerSecRx))
            metricLabel("Frames", value: "\(client.telemetry.framesReceived)")
            metricLabel("Ratio", value: String(format: "%.1fx", client.telemetry.compressionRatio))
            Spacer()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color.secondary.opacity(0.12))
        )
    }

    @ViewBuilder
    private var surfaceBody: some View {
        if let frame = client.latestFrame {
            if let frameBuilder {
                frameBuilder(client.latestFrameSeq, frame)
            } else {
                defaultCommandBufferDisplay(seq: client.latestFrameSeq, frame: frame)
            }
        } else {
            VStack(spacing: 6) {
                Text(client.state.isPaired ? "Waiting for first NVD1 keyframe…" : "Not paired")
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
                Text("Zero-cdylib NVT1/NVD1 thin client")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            }
            .frame(maxWidth: .infinity, minHeight: 140)
            .background(
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(Color.secondary.opacity(0.25), style: StrokeStyle(lineWidth: 1, dash: [5]))
            )
        }
    }

    private func defaultCommandBufferDisplay(seq: UInt32, frame: Data) -> some View {
        let summary = client.latestSummary
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Live Paired CommandBuffer")
                    .font(.caption.weight(.semibold))
                Spacer()
                Text("seq #\(seq) · \(frame.count) B")
                    .font(.system(.caption2, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            if let summary {
                HStack(spacing: 14) {
                    metricLabel("Surface", value: "\(summary.surfaceId)")
                    metricLabel("Frame", value: "\(summary.frameId)")
                    metricLabel("Ops", value: "\(summary.opCount)")
                    metricLabel("Flags", value: String(format: "0x%04X", summary.flags))
                }
            } else {
                Text("Raw binary snapshot (\(frame.count) bytes)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            Text("Tap surface to send InputEvents=11 press")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, minHeight: 140, alignment: .topLeading)
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(Color.accentColor.opacity(0.08))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(Color.accentColor.opacity(0.3), lineWidth: 1)
                )
        )
        .contentShape(Rectangle())
        .modifier(NvePairedTapModifier(surfaceId: summary?.surfaceId ?? surfaceId) { x, y in
            let event = NveEvent(
                surfaceId: summary?.surfaceId ?? surfaceId,
                nodeId: 1,
                timestampUs: UInt64(Date().timeIntervalSince1970 * 1_000_000),
                kind: .press,
                payload: .point(x: x, y: y)
            )
            _ = try? client.sendInputEvents([event])
        })
    }

    private func metricLabel(_ title: String, value: String) -> some View {
        HStack(spacing: 4) {
            Text(title)
                .font(.caption2)
                .foregroundStyle(.secondary)
            Text(value)
                .font(.system(.caption2, design: .monospaced).weight(.semibold))
        }
    }

    private var isActiveConnection: Bool {
        switch client.state {
        case .connecting, .paired, .reconnecting:
            return true
        case .disconnected, .error:
            return false
        }
    }

    private var statusColor: Color {
        switch client.state {
        case .disconnected: return .gray
        case .connecting, .reconnecting: return .orange
        case .paired: return .green
        case .error: return .red
        }
    }

    private var formattedRtt: String {
        guard let ms = client.telemetry.smoothedRttMs else { return "—" }
        return String(format: "%.1fms", ms)
    }

    private func formattedThroughput(_ bytesPerSec: Double) -> String {
        if bytesPerSec >= 1024.0 {
            return String(format: "%.1f KiB/s", bytesPerSec / 1024.0)
        }
        return String(format: "%.0f B/s", bytesPerSec)
    }

    private func toggleConnection() {
        validationError = nil
        if isActiveConnection {
            client.disconnect()
            return
        }
        do {
            let fallback = tokenInput.trimmingCharacters(in: .whitespacesAndNewlines)
            try client.connect(uriString: uriInput, fallbackToken: fallback.isEmpty ? nil : fallback)
        } catch {
            validationError = String(describing: error)
        }
    }
}

public extension NvePairedSurfaceView where Content == EmptyView {
    init(
        client: NvePairClient,
        initialUri: String = "nve://pair?host=127.0.0.1&ws=8092&token=dev",
        initialToken: String = "",
        surfaceId: UInt32 = 1
    ) {
        self.client = client
        self._uriInput = State(initialValue: initialUri)
        self._tokenInput = State(initialValue: initialToken)
        self.surfaceId = surfaceId
        self.frameBuilder = nil
    }
}

private struct NvePairedTapModifier: ViewModifier {
    let surfaceId: UInt32
    let onTap: (Float, Float) -> Void

    func body(content: Content) -> some View {
        #if os(tvOS)
        content.onTapGesture {
            onTap(0, 0)
        }
        #else
        content.gesture(
            SpatialTapGesture()
                .onEnded { value in
                    onTap(Float(value.location.x), Float(value.location.y))
                }
        )
        #endif
    }
}
