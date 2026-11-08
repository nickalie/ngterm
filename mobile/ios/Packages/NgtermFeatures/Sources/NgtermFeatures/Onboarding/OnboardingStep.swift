import NgtermPlatform

/// Where the first run is.
public enum OnboardingStep: Hashable, Sendable {
    /// The app has a key: the first run is over.
    case done
    /// The user is importing a key; `isBusy` while it is read, `problem` what was wrong with the last attempt.
    case importing(isBusy: Bool, problem: ImportProblem?)
    /// A key is ready: its public half is shown, to put on the machines.
    case keyReady(AppKey)
    /// The app's keys couldn't be read, for `reason` (the system's words): whether it has one is unknown until a retry
    /// reads them.
    case readFailed(reason: String)
    /// No key yet: make one or bring one; `isBusy` while one is being made, `failure` why the last attempt failed.
    case welcome(isBusy: Bool, failure: String?)

    /// The welcome, nothing under way.
    public static let start = Self.welcome(isBusy: false, failure: nil)
}
