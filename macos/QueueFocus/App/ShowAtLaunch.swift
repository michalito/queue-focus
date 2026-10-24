#if DEBUG
import AppKit

/// Views opened at launch, to look at them by eye without driving the app:
/// `-showAtLaunch queue,board,settings,add,popover`, one or more, and
/// `-showAtLaunchHeight` to make the windows taller. Debug
/// builds only. With `XDG_DATA_HOME` on a folder of its own and
/// `-allowSecondInstance YES`, a development build shows a made-up queue
/// beside the installed app.
@MainActor
enum ShowAtLaunch {
    static func schedule(_ defaults: UserDefaults, windows: AppWindows, quickAdd: QuickAddController?,
                         popover: @escaping @MainActor () -> Void) {
        guard let list = defaults.string(forKey: "showAtLaunch") else { return }
        let views = list.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
        // The window actions are lent once the status item is in a window.
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
            MainActor.assumeIsolated {
                for view in views {
                    switch view {
                    case "queue": windows.show(WindowID.queue)
                    case "board": windows.show(WindowID.board)
                    case "settings": windows.show(WindowID.settings)
                    case "add": quickAdd?.show()
                    case "popover": popover()
                    default: break
                    }
                }
                // `-showAtLaunchHeight 1600`: every window as tall as that,
                // to see the whole of Settings at once.
                let height = defaults.double(forKey: "showAtLaunchHeight")
                guard height > 0 else { return }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    for window in NSApp.windows where window.isVisible && window.styleMask.contains(.titled) {
                        var frame = window.frame
                        frame.origin.y += frame.height - height
                        frame.size.height = height
                        window.setFrame(frame, display: true)
                    }
                }
            }
        }
    }
}
#endif
