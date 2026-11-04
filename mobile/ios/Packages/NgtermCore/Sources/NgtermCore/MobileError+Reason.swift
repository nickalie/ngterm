import NgtermCoreBindings

extension MobileError {
    /// The words for the failure, as the core gives them: what the app shows under a machine or over a listing.
    public var reason: String {
        switch self {
        case .Unreachable(let reason), .Remote(let reason):
            reason

        case .Cancelled:
            "the call was cancelled"

        case .Stopped:
            "the core has stopped"

        case .NotConnected(let machineId):
            "\(machineId) is not connected"

        case .NotListed(let sessionId):
            "\(sessionId) is not in the machine's last listing"
        }
    }
}
