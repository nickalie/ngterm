import NgtermDesign
import PhotosUI
import SwiftUI

/// Attach, inside the pill (mobile/android's `AttachButton`): the paperclip opens a menu of the system's pickers (Photo
/// or video, the photo picker; File, the document picker), neither needing a permission; what the user picks goes to
/// `onPick`.
///
/// It holds while an upload is on its way.
internal struct AttachButton: View {
    internal let isEnabled: Bool
    internal let onPick: @MainActor (PickedFile) -> Void
    @State private var isPickingPhoto = false
    @State private var isPickingFile = false
    @State private var photo: PhotosPickerItem?

    internal var body: some View {
        Menu {
            Button(String(localized: "Photo or video"), systemImage: "photo") {
                isPickingPhoto = true
            }
            Button(String(localized: "File"), systemImage: "doc") {
                isPickingFile = true
            }
        } label: {
            Image(systemName: ChatStyle.attachSymbol)
                .font(.body)
                .frame(width: Spacing.hitTarget, height: Spacing.hitTarget)
                .contentShape(Rectangle())
        }
        .foregroundStyle(Palette.secondaryText.color)
        .disabled(!isEnabled)
        .accessibilityLabel(String(localized: "Attach a file"))
        .accessibilityIdentifier("chat-attach")
        .photosPicker(
            isPresented: $isPickingPhoto,
            selection: $photo,
            matching: .any(of: [.images, .videos]),
            preferredItemEncoding: .compatible
        )
        .onChange(of: photo) { _, item in
            if let item {
                photo = nil
                onPick(Self.picked(item))
            }
        }
        .fileImporter(isPresented: $isPickingFile, allowedContentTypes: [.item]) { result in
            if case .success(let url) = result {
                onPick(Self.picked(url))
            }
        }
    }

    // a photo or video from the photo picker: named for its kind, as the picker gives no file name
    private static func picked(_ item: PhotosPickerItem) -> PickedFile {
        let type = item.supportedContentTypes.first
        let ext = type?.preferredFilenameExtension ?? "jpg"
        let stem = type?.conforms(to: .movie) == true ? "video" : "photo"
        return PickedFile(name: "\(stem).\(ext)", size: nil) {
            // a file, not bytes: a long video would fill the memory before it could be refused as too large
            guard let media = try await item.loadTransferable(type: PickedMedia.self) else {
                throw UploadProblem.unreadable
            }
            return try media.read()
        }
    }

    // a file from the document picker: its own name and size, read while its security scope is open, no more than
    // one byte past the cap, so a file whose size wasn't known still can't fill the memory before it is refused
    private static func picked(_ url: URL) -> PickedFile {
        let size = (try? url.resourceValues(forKeys: [.fileSizeKey]))?.fileSize
        return PickedFile(name: url.lastPathComponent, size: size) {
            let isScoped = url.startAccessingSecurityScopedResource()
            defer {
                if isScoped {
                    url.stopAccessingSecurityScopedResource()
                }
            }
            let handle = try FileHandle(forReadingFrom: url)
            defer { try? handle.close() }
            return try handle.read(upToCount: ChatUpload.cap + 1) ?? Data()
        }
    }
}
