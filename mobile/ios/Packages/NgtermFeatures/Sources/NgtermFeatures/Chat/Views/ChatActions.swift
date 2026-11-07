/// What a chat does, besides its model's own calls: show the pane's terminal, send, attach a picked file.
internal struct ChatActions {
    internal let onShowTerminal: @MainActor () -> Void
    internal let onSend: @MainActor () -> Void
    internal let onAttach: @MainActor (PickedFile) -> Void
    internal let onCancelUpload: @MainActor () -> Void
    internal let onUploadShown: @MainActor () -> Void
}
