import NgtermDesign
import Testing
import UIKit

/// The bundled fonts register, draw under their names, and ship with their licenses.
@Suite("fonts")
internal struct FontsTests {
    @Test("every bundled font registers")
    internal func everyFontRegisters() {
        #expect(Fonts.unregistered.isEmpty)
    }

    @Test("the terminal's four faces and the mono family are there to draw with")
    internal func facesResolve() {
        Fonts.register()
        for face in Fonts.terminalFaces.all {
            #expect(UIFont(name: face, size: 13)?.fontName == face)
        }
        let mono = UIFont(name: Fonts.monoRegular, size: 13)
        #expect(mono?.familyName == Fonts.monoFamily)
        let traits = mono?.fontDescriptor.object(forKey: .traits) as? [UIFontDescriptor.TraitKey: Any]
        let weight = traits?[.weight] as? CGFloat
        #expect(weight == UIFont.Weight.regular.rawValue, "the regular instance, not the extra light default")
    }

    @Test("the fonts' licenses ship beside them")
    internal func licensesShip() {
        for license in Fonts.licenses {
            #expect(Fonts.url(of: license) != nil, "\(license)")
        }
    }
}
