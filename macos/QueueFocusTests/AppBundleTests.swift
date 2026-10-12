import Foundation
import Testing

/// What the build puts in the app's bundle.
@Suite struct AppBundleTests {
    /// Xcode compiles `AppIcon.icon` into layers in Assets.car, for macOS 26 on, and a flat
    /// AppIcon.icns for macOS 14 and 15; without the setting naming it, the app keeps the
    /// generic icon and builds without a word.
    @Test func theAppHasItsIcon() {
        #expect(Bundle.main.object(forInfoDictionaryKey: "CFBundleIconName") as? String == "AppIcon")
        #expect(Bundle.main.object(forInfoDictionaryKey: "CFBundleIconFile") as? String == "AppIcon")
        #expect(Bundle.main.url(forResource: "AppIcon", withExtension: "icns") != nil)
        #expect(Bundle.main.url(forResource: "Assets", withExtension: "car") != nil)
    }
}
