import Foundation
import NveSwiftUI
import XCTest

final class NveChannelClientTests: XCTestCase {
    func testPublishSubscribeAndTopicIsolation() {
        let client = NveChannelClient()
        var themeMessages: [NveChannelClient.Message] = []
        var authMessages: [NveChannelClient.Message] = []

        client.subscribe(topic: "theme") { themeMessages.append($0) }
        client.subscribe(topic: "auth") { authMessages.append($0) }

        client.publish(topic: "theme", payload: Data("dark".utf8))
        client.publish(topic: "auth", payload: Data("token-1".utf8))
        client.publish(topic: "theme", payload: Data("light".utf8))

        XCTAssertEqual(themeMessages.count, 2)
        XCTAssertEqual(authMessages.count, 1)

        XCTAssertEqual(themeMessages[0].topic, "theme")
        XCTAssertEqual(String(data: themeMessages[0].payload, encoding: .utf8), "dark")
        XCTAssertEqual(themeMessages[0].seq, 1)

        XCTAssertEqual(authMessages[0].topic, "auth")
        XCTAssertEqual(authMessages[0].seq, 2)

        XCTAssertEqual(String(data: themeMessages[1].payload, encoding: .utf8), "light")
        XCTAssertEqual(themeMessages[1].seq, 3)
    }

    func testRetainedReplayAndOptOut() throws {
        let client = NveChannelClient()
        client.publishJSON(topic: "theme", object: ["mode": "dark"], retain: true)
        client.publishJSON(topic: "theme", object: ["mode": "transient"], retain: false)

        var replayed: [NveChannelClient.Message] = []
        client.subscribe(topic: "theme", replayRetained: true) { replayed.append($0) }
        XCTAssertEqual(replayed.count, 1)
        let json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: replayed[0].payload) as? [String: Any]
        )
        XCTAssertEqual(json["mode"] as? String, "dark")
        XCTAssertEqual(replayed[0].seq, 1)

        var noReplay: [NveChannelClient.Message] = []
        client.subscribe(topic: "theme", replayRetained: false) { noReplay.append($0) }
        XCTAssertTrue(noReplay.isEmpty)

        // Overwrite retained value
        client.publishJSON(topic: "theme", object: ["mode": "high-contrast"], retain: true)
        XCTAssertEqual(replayed.count, 2)
        XCTAssertEqual(noReplay.count, 1)

        var lateSubscriber: [NveChannelClient.Message] = []
        client.subscribe(topic: "theme", replayRetained: true) { lateSubscriber.append($0) }
        XCTAssertEqual(lateSubscriber.count, 1)
        let latestJson = try XCTUnwrap(
            JSONSerialization.jsonObject(with: lateSubscriber[0].payload) as? [String: Any]
        )
        XCTAssertEqual(latestJson["mode"] as? String, "high-contrast")
    }

    func testUnsubscribeStopsOnlyTargetedListener() {
        let client = NveChannelClient()
        var firstCount = 0
        var secondCount = 0

        let token1 = client.subscribe(topic: "navigation") { _ in firstCount += 1 }
        _ = client.subscribe(topic: "navigation") { _ in secondCount += 1 }

        client.publish(topic: "navigation", payload: Data([1]))
        XCTAssertEqual(firstCount, 1)
        XCTAssertEqual(secondCount, 1)

        client.unsubscribe(topic: "navigation", token: token1)
        client.unsubscribe(topic: "nonexistent", token: UUID())

        client.publish(topic: "navigation", payload: Data([2]))
        XCTAssertEqual(firstCount, 1)
        XCTAssertEqual(secondCount, 2)
    }

    func testConcurrentPublishAndSubscribeThreadSafety() {
        let client = NveChannelClient()
        let countLock = NSLock()
        var receivedSeqs: [UInt64] = []

        _ = client.subscribe(topic: "metrics") { msg in
            countLock.lock()
            receivedSeqs.append(msg.seq)
            countLock.unlock()
        }

        let iterations = 100
        DispatchQueue.concurrentPerform(iterations: iterations) { idx in
            let token = client.subscribe(topic: "ephemeral-\(idx % 4)") { _ in }
            client.publish(topic: "metrics", payload: Data("\(idx)".utf8), retain: idx % 10 == 0)
            client.unsubscribe(topic: "ephemeral-\(idx % 4)", token: token)
        }

        countLock.lock()
        let total = receivedSeqs.count
        let unique = Set(receivedSeqs).count
        countLock.unlock()

        XCTAssertEqual(total, iterations)
        XCTAssertEqual(unique, iterations)
    }
}
