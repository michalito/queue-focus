# GNOME Shell 46 compatibility investigation

Investigated 2026-10-20 against this checkout and the machine's installed GNOME Shell **46.3.1**. This note covers the Shell extension; GTK app/build requirements are separate.

## Finding

**The only demonstrated extension change needed is adding `"46"` to `metadata.json`'s `shell-version` array, keeping `48`, `49`, and `50`.** No JavaScript backport was necessary for the checks below. The shipped extension passed the existing real-Shell integration suite on this machine in a disposable copy with that metadata change alone. An additional temporary driver check exercised the actual notification constructor/action and all six flash styles with animations enabled and disabled, and also passed.

The current exclusion is at `extension/queue-focus@queuefocus.org/metadata.json:5`. GNOME's documentation says this array determines supported major releases and version validation is enabled by default. Add the version after testing rather than disabling validation globally. Do not infer GNOME 47 support without testing that release. [GNOME extension metadata documentation](https://gjs.guide/extensions/overview/anatomy.html#shell-version)

## API audit and exact code locations

- **Imports and extension lifecycle:** `extension.js:19` already uses the ES module `Extension` API introduced before 46. No legacy-import rewrite is needed. [GNOME 45 porting guide](https://gjs.guide/extensions/upgrading/gnome-shell-45.html)
- **Vertical layouts:** `extension.js:84` and `flash.js:82` already feature-test `orientation` and fall back to `vertical = true`. The 46.3.1 implementation exports the boolean property. Retain both branches for older and newer releases. The comments saying the fallback is for 48 should be broadened to include 46; no logic change is required. [GNOME Shell 46.3.1 St.BoxLayout source](https://raw.githubusercontent.com/GNOME/gnome-shell/46.3.1/src/st/st-box-layout.c)
- **Scroll child:** `extension.js:553` assigns `scroll.child = list`. GNOME 46.3.1 already has a writable `St.ScrollView:child` property backed by `st_scroll_view_set_child`, so reverting to obsolete `add_actor` is unnecessary. [GNOME Shell 46.3.1 St.ScrollView source](https://raw.githubusercontent.com/GNOME/gnome-shell/46.3.1/src/st/st-scroll-view.c)
- **Notifications:** `extension.js:356` uses `new MessageTray.Source({title, iconName})`, `new MessageTray.Notification({source, title, body, isTransient})`, `addAction`, and `addNotification`. GNOME 46 already has options-object constructors (`Notification` near upstream line 330, `Source` near 456) and both methods. A backward shim using positional constructors would be wrong. The temporary smoke check successfully constructed an actual notification with an Undo action and destroyed it. [GNOME Shell 46.3.1 messageTray source](https://raw.githubusercontent.com/GNOME/gnome-shell/46.3.1/js/ui/messageTray.js)
- **Fullscreen flash compositing:** `flash.js:98` already chooses `Meta.disable_unredirect_for_display(global.display)` / `enable_unredirect_for_display` when available and otherwise uses `global.compositor`. Mutter 46 defines the former pair; retain this fallback to preserve 46 and the newer namespace. [Mutter 46.3 compositor source](https://raw.githubusercontent.com/GNOME/mutter/46.3/src/compositor/compositor.c), [GNOME 48 Meta API changes](https://gjs.guide/extensions/upgrading/gnome-shell-48.html#meta)
- **Color API:** no production extension file references `Clutter.Color` or `Cogl.Color`; flash colors use CSS `rgba(...)` strings (`flash.js:20`, `:224`, `:299`). The 47 change removing `Clutter.Color` is therefore irrelevant to this implementation. [GNOME 47 color API change](https://gjs.guide/extensions/upgrading/gnome-shell-47.html#clutter-color)
- **Flash picking, layout, animation, cleanup:** `flash.js:149` calls `Shell.util_set_hidden_from_pick`, followed by real actors/animations/timers. Installed `/usr/share/gnome-shell/Shell-14.gir:5639` exposes the helper. All six styles constructed actors and cleared themselves in the temporary real-Shell check, in both animation modes. This checks actual API availability, not appearance over a fullscreen client.
- **Timers:** `extension.js:208` uses `GLib.timeout_add_seconds`; undo uses `GLib.timeout_add` (`:314`), as do the nonanimated flash (`flash.js:173`) and D-Bus retries (`dbus.js:55`). These APIs predate GNOME 46; no newer one-shot GJS timer helper is used. The flash test actually exercised its timer cleanup; the suite exercised retry callbacks and timer enable/pause behavior, but did not wait for a real minute rollover or the complete eight-second undo expiry. [GLib timeout_add](https://docs.gtk.org/glib/func.timeout_add.html), [GLib timeout_add_seconds](https://docs.gtk.org/glib/func.timeout_add_seconds.html)
- **Shortcuts:** `extension.js:648` registers four settings-backed actions with `Main.wm.addKeybinding(name, settings, flags, modes, handler)` and removes them in `disable()`. GNOME 46 has that exact signature and the same `IGNORE_AUTOREPEAT` / `NORMAL | OVERVIEW` pattern. Successful disable/re-enable exercises registration and cleanup, but does not prove keyboard dispatch or absence of a user's shortcut conflicts. [GNOME Shell 46.3.1 WindowManager source](https://raw.githubusercontent.com/GNOME/gnome-shell/46.3.1/js/ui/windowManager.js)

## Empirical checks

No software was installed. No application source, installed extension, task data, or live desktop settings were changed. Only this research note was added to the checkout.

`gnome-shell --version` returned `GNOME Shell 46.3.1`. Its `--help` includes every launch flag used by `scripts/test-extension-shell.sh:53`: `--headless --wayland --no-x11 --virtual-monitor`. Mutter 46's own option parser also implements these flags. The existing harness therefore needs no GNOME 46 launcher adaptation. [Mutter 46.3 context/option parser](https://raw.githubusercontent.com/GNOME/mutter/46.3/src/core/meta-context-main.c)

The first check used this reproducible procedure from the repo root:

```sh
python3 - <<'PY'
import json, pathlib, shutil, subprocess, tempfile
root = pathlib.Path.cwd()
with tempfile.TemporaryDirectory(prefix='qf-46-research-') as directory:
    temporary = pathlib.Path(directory)
    shutil.copytree(root / 'extension', temporary / 'extension')
    (temporary / 'scripts').mkdir()
    shutil.copy2(root / 'scripts/test-extension-shell.sh',
                 temporary / 'scripts/test-extension-shell.sh')
    metadata = temporary / 'extension/queue-focus@queuefocus.org/metadata.json'
    data = json.loads(metadata.read_text())
    data['shell-version'].insert(0, '46')
    metadata.write_text(json.dumps(data))
    subprocess.run(['bash', str(temporary / 'scripts/test-extension-shell.sh')],
                   check=True, timeout=80)
PY
```

**Result: exit 0.** The harness reported success for quick-add draft preservation, late replies, disconnect/reconnect, disable/re-enable, virtual pointer clock pausing without opening the menu, focus card, Side cards, done/undo/promotion, view requests, long titles, and constrained panel/menu width. It also rejects extension stack traces and inconsistent size requests (`scripts/test-extension-shell.sh:69–78`). The launcher creates private XDG directories and a private D-Bus without service activation directories (`:8–40`), so its fixture cannot activate the installed app or access the user's queue.

The second check used the same disposable layout with extra test-only code appended after `shell-driver.js:321`. It called the real indicator's `_notify(...)` with an Undo action, checked one registered action, and destroyed the notification. It then changed the disposable `org.gnome.desktop.interface` animation setting to false and true; for each value it called the real `_flash.show(...)` with each of `wash`, `wash2`, `edges`, `edgesSoft`, `topbar`, `topbarBeam`, asserted an overlay existed, and awaited its automatic removal. **Result: exit 0**, including the suite's log checks. These smoke additions were not retained as application changes.

## Recommended implementation and remaining verification

1. Add `46` to metadata while retaining the existing supported releases; update compatibility comments and supported-version documentation.
2. Retain the current feature-detection branches. The evidence does not justify a second extension version or broad notification/layout rewrite.
3. Include a real GNOME 46 environment in the compatibility checks. The current private headless harness already works there. Add the notification/flash coverage permanently if maintaining these paths across Shell releases.
4. Once the separate GTK/build changes are made, test the actual application/service and extension together on Ubuntu 24.04. The isolated fixture validates the extension, not GTK rendering, binary linkage, installation, or D-Bus activation of the Rust application.
5. Check a real desktop session for Yaru appearance, configured accelerators, minute rollover, notification action dispatch, fullscreen flash visibility, fractional scaling, and other extensions. The headless run used a 1280×720 virtual monitor and software rendering. No claim of a finished user installation follows from these research probes, and GNOME 48–50 were not re-tested on this machine.
