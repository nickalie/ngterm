import UIKit

/// Tells a ``Lifecycle`` which of the app's scenes come to the foreground and leave it, from UIKit's scene
/// notifications: any scene, whichever window it is.
@preconcurrency
@MainActor
public final class SceneWatcher {
    private var observers: [any NSObjectProtocol] = []

    /// Starts telling `lifecycle` from now on.
    public init(lifecycle: Lifecycle) {
        let center = NotificationCenter.default
        let entered = [UIScene.willEnterForegroundNotification]
        let left = [UIScene.didEnterBackgroundNotification, UIScene.didDisconnectNotification]
        for name in entered {
            observers.append(
                center.addObserver(forName: name, object: nil, queue: .main) { [weak lifecycle] note in
                    let scene = ObjectIdentifier(note.object as AnyObject)
                    MainActor.assumeIsolated {
                        lifecycle?.sceneEntered(scene)
                    }
                }
            )
        }
        for name in left {
            observers.append(
                center.addObserver(forName: name, object: nil, queue: .main) { [weak lifecycle] note in
                    let scene = ObjectIdentifier(note.object as AnyObject)
                    MainActor.assumeIsolated {
                        lifecycle?.sceneLeft(scene)
                    }
                }
            )
        }
    }

    isolated deinit {
        for observer in observers {
            NotificationCenter.default.removeObserver(observer)
        }
    }
}
