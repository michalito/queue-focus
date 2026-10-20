import Foundation
import Testing

/// What the build puts in the app's bundle.
@Suite struct AppBundleTests {
    /// Xcode compiles `AppIcon.icon` into Assets.car, as the layers macOS 26 on draws and the
    /// flat sizes earlier versions draw, and puts a few sizes in AppIcon.icns for tools that
    /// cannot read Assets.car. Without the setting naming it, the app keeps the generic icon
    /// and builds without a word.
    @Test func theAppHasItsIcon() {
        #expect(Bundle.main.object(forInfoDictionaryKey: "CFBundleIconName") as? String == "AppIcon")
        #expect(Bundle.main.object(forInfoDictionaryKey: "CFBundleIconFile") as? String == "AppIcon")
        #expect(Bundle.main.url(forResource: "AppIcon", withExtension: "icns") != nil)
        #expect(Bundle.main.url(forResource: "Assets", withExtension: "car") != nil)
    }
}
