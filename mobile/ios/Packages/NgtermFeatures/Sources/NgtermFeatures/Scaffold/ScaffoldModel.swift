import Observation
import SwiftUI

/// A window's navigation: the machine and the session it shows, which column a compact width shows, and the Settings
/// sheet and the onboarding cover.
///
/// It lives as long as its scene, so a size-class change (rotation, Stage Manager, Split View) keeps it whole.
@preconcurrency
@MainActor
@Observable
public final class ScaffoldModel {
    /// The machine whose sessions show, by its id in the app's store.
    public var machine: String?
    /// The session that shows: setting it takes it from any other main window, which then shows it no more.
    public var session: SessionWindowID? {
        didSet {
            windows.show(session, in: ObjectIdentifier(self))
        }
    }
    /// The column a compact width shows: the stack's top.
    public var compactColumn: NavigationSplitViewColumn = .sidebar
    /// Which columns a regular width shows: all three where they fit.
    ///
    /// An iPad held upright has room for two, so the machines start hidden there, and the empty sessions column
    /// offers Show machines.
    public var columnVisibility: NavigationSplitViewVisibility = .all
    /// Settings is open.
    public var isSettingsPresented = false
    /// The first run's onboarding covers the app: there is nowhere to go without a key.
    public var isOnboardingPresented: Bool
    /// Where every session is drawn, shared by every scene.
    public let windows: SessionWindows

    /// Where the session this window shows is drawn: here, unless it has a window of its own by now, or another main
    /// window took it over.
    public var shownSessionPlacement: SessionPlacement? {
        session.map { windows.placement(of: $0, in: ObjectIdentifier(self)) }
    }

    /// The machines' column is hidden (an iPad held upright, or the user hid it), so Show machines has something to
    /// do.
    public var areMachinesHidden: Bool {
        columnVisibility != .all
    }

    /// A window showing nothing yet, the onboarding over it when `needsOnboarding`.
    public init(windows: SessionWindows, needsOnboarding: Bool = false) {
        self.windows = windows
        isOnboardingPresented = needsOnboarding
    }

    /// Shows the sessions of `machine`, and no session of another machine.
    public func show(machine: String) {
        if self.machine != machine {
            session = nil
        }
        self.machine = machine
        compactColumn = .content
    }

    /// Opens `session` here, taking it from another main window that shows it, or says it has its own window: the
    /// caller then brings that window forward.
    public func open(_ session: SessionWindowID) -> SessionPlacement {
        if windows.placement(of: session, in: ObjectIdentifier(self)) == .ownWindow {
            return .ownWindow
        }
        machine = session.machineID
        self.session = session
        compactColumn = .detail
        return .here
    }

    isolated deinit {
        // the scene went away: another main window may show its session now
        windows.show(nil, in: ObjectIdentifier(self))
    }
}
