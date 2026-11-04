/// The terminal's font: the bundled JetBrains Mono (desktop ngterm's and Ghostty's), or the system's monospace.
public enum FontChoice: StoredChoice {
    /// The bundled JetBrains Mono.
    case jetBrainsMono
    /// The system's monospaced face (SF Mono).
    case systemMono
}
