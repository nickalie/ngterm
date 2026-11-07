import Foundation
import NgtermPlatform
import Testing
import UIKit

/// The scene watcher: UIKit's scene notifications, from any scene, reach the lifecycle as scenes entering and leaving
/// the foreground.
@MainActor
@Suite("the scene watcher")
internal struct SceneWatcherTests {
    /// What the lifecycle did, holding nothing.
    @MainActor
    internal final class Record {
        internal private(set) var suspends = 0
        internal private(set) var resumes = 0

        internal func suspend() {
            suspends += 1
        }

        internal func resume() {
            resumes += 1
        }

        deinit {
            // nothing to release
        }
    }

    private static func post(_ name: Notification.Name, _ scene: NSObject) {
        NotificationCenter.default.post(name: name, object: scene)
    }

    private static func lifecycle(_ record: Record) -> Lifecycle {
        Lifecycle(grace: FakeGrace(), holds: { .nothing }, suspend: record.suspend, resume: record.resume)
    }

    @Test("a scene entering the foreground and leaving it to the background reaches the lifecycle")
    internal func enterAndBackground() {
        let record = Record()
        let lifecycle = Self.lifecycle(record)
        let watcher = SceneWatcher(lifecycle: lifecycle)
        let scene = NSObject()
        Self.post(UIScene.willEnterForegroundNotification, scene)
        #expect(lifecycle.isInForeground)
        Self.post(UIScene.didEnterBackgroundNotification, scene)
        #expect(!lifecycle.isInForeground)
        #expect(record.suspends == 1)
        Self.post(UIScene.willEnterForegroundNotification, scene)
        #expect(record.resumes == 1)
        withExtendedLifetime(watcher) {
            // the watcher tells the lifecycle for as long as it is kept
        }
    }

    @Test("each scene counts: a disconnected one leaves the foreground, and the last one leaving lets go")
    internal func scenesCountApart() {
        let record = Record()
        let lifecycle = Self.lifecycle(record)
        let watcher = SceneWatcher(lifecycle: lifecycle)
        let first = NSObject()
        let second = NSObject()
        Self.post(UIScene.willEnterForegroundNotification, first)
        Self.post(UIScene.willEnterForegroundNotification, second)
        Self.post(UIScene.didDisconnectNotification, first)
        #expect(lifecycle.isInForeground)
        #expect(record.suspends == 0)
        Self.post(UIScene.didDisconnectNotification, second)
        #expect(!lifecycle.isInForeground)
        #expect(record.suspends == 1)
        withExtendedLifetime(watcher) {
            // the watcher tells the lifecycle for as long as it is kept
        }
    }

    @Test("a watcher gone tells the lifecycle nothing more")
    internal func goneWatcherIsQuiet() {
        let record = Record()
        let lifecycle = Self.lifecycle(record)
        var watcher: SceneWatcher? = SceneWatcher(lifecycle: lifecycle)
        let scene = NSObject()
        Self.post(UIScene.willEnterForegroundNotification, scene)
        watcher = nil
        Self.post(UIScene.didEnterBackgroundNotification, scene)
        #expect(watcher == nil)
        #expect(lifecycle.isInForeground)
        #expect(record.suspends == 0)
    }
}
