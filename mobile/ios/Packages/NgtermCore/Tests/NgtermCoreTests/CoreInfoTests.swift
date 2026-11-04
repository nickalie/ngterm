import NgtermCore
import NgtermCoreFakes
import Testing

@Suite("CoreInfo")
internal struct CoreInfoTests {
    @Test("the linked core reports its workspace version")
    internal func linkedCoreReportsItsVersion() {
        let version = CoreInfo.version
        #expect(version.wholeMatch(of: /\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?/) != nil, "core_version() gave \(version)")
    }

    @Test("the fake core is compiled into Debug builds and tells itself apart from the real one")
    internal func fakeCoreIsNotTheRealOne() {
        #expect(FakeCoreInfo.version != CoreInfo.version)
    }
}
