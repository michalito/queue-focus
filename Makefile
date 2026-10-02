# queue-focus — common tasks. `make help` lists them.
CARGO   := scripts/cargo
UUID    := queue-focus@queuefocus.org
SCHEMAS := extension/$(UUID)/schemas
# Only explicit command-line values affect version files. Ambient environment
# variables named VERSION must never make a plain `make install` mutate them.
REQUESTED_VERSION           := $(if $(filter command line,$(origin VERSION)),$(VERSION),)
REQUESTED_EXTENSION_VERSION := $(if $(filter command line,$(origin EXTENSION_VERSION)),$(EXTENSION_VERSION),)
export REQUESTED_VERSION REQUESTED_EXTENSION_VERSION

.PHONY: help build test test-core test-mac-core test-mac-app test-mac-ui release-mac release-mac-dry-run test-ui test-service check-core mac-core mac-app test-install test-version test-extension test-extension-dbus test-extension-shell check version set-version maybe-version install uninstall update deb clean run

help:            ## show this help
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | awk -F':.*##' '{printf "  %-13s %s\n", $$1, $$2}'

build: $(SCHEMAS)/gschemas.compiled   ## release build (no install)
	$(CARGO) build --release -p queue-focus

$(SCHEMAS)/gschemas.compiled: $(SCHEMAS)/*.gschema.xml
	glib-compile-schemas --strict $(SCHEMAS)

test:            ## unit + isolated integration tests
	$(CARGO) test --workspace
	node extension/test/flash.test.mjs
	node extension/test/connection.test.mjs
	scripts/test-install-local.sh
	scripts/test-set-version.sh

test-core:       ## engine and FFI tests; runs on Linux and macOS
	$(CARGO) test -p qf-core -p qf-ffi

test-mac-core: mac-core  ## Swift tests of the engine through its bindings (macOS)
	cd macos/QfCore && swift test

test-ui:         ## real GTK placement/drop/focus test (needs Xvfb)
	scripts/test-ui.sh

test-service:    ## the real service over D-Bus and the command line (needs Xvfb)
	scripts/test-service.sh

test-install:    ## isolated local installer integration tests
	scripts/test-install-local.sh

test-version:    ## isolated versioning integration tests
	scripts/test-set-version.sh

test-extension:  ## shell-extension tests, against a stubbed GNOME Shell
	node extension/test/flash.test.mjs
	node extension/test/connection.test.mjs

test-extension-dbus: ## real GJS adapter test on a private D-Bus (needs gjs)
	GIO_USE_VFS=local dbus-run-session -- gjs -m extension/test/dbus.test.js

test-extension-shell: ## real quick-add, menu and width tests in a disposable headless GNOME Shell
	scripts/test-extension-shell.sh

check:           ## fmt + clippy + JS/Python/shell syntax
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	for js in extension/$(UUID)/*.js; do node --check "$$js" || exit 1; done
	for script in scripts/*.sh; do bash -n "$$script" || exit 1; done
	python3 -c 'from pathlib import Path; compile(Path("scripts/set-version").read_text(), "scripts/set-version", "exec")'

check-core:      ## fmt + clippy for the crates the macOS app uses; runs on Linux and macOS
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy -p qf-core -p qf-ffi -p uniffi-bindgen --all-targets -- -D warnings

mac-core:        ## build the engine's XCFramework and Swift module for the macOS app
	scripts/build-mac-core.sh --if-changed

# The macOS app, built and tested in Debug for this Mac's architecture, as
# Xcode's own Debug build does, so the two share one build of the engine.
# Swift packages come at the versions in Package.resolved and are not looked
# up again once fetched.
MAC_ARCH := $(shell uname -m)
XCODEBUILD = xcodebuild -project macos/QueueFocus.xcodeproj -scheme QueueFocus \
	-configuration Debug -derivedDataPath target/xcode \
	-onlyUsePackageVersionsFromResolvedFile -skipPackageUpdates

mac-app:         ## build the macOS app into target/xcode/Build/Products/Debug
	scripts/build-mac-core.sh --archs $(MAC_ARCH) --if-changed
	$(XCODEBUILD) build

# On a build machine (CI set), the hosted tests hold the app to a clean Mac:
# a default shortcut the system holds fails rather than being reported.
ifdef CI
export TEST_RUNNER_CI := 1
endif

# The test plan keeps no screenshots or screen recordings: a UI test's would
# show whatever else is on the screen.
XCODETEST = $(XCODEBUILD) test -testPlan QueueFocus -collect-test-diagnostics never

test-mac-app:    ## the macOS app's unit tests
	scripts/build-mac-core.sh --archs $(MAC_ARCH) --if-changed
	$(XCODETEST) -only-testing:QueueFocusTests

test-mac-ui:     ## the macOS app's UI tests; they drive the real menu bar, pointer and keyboard
	scripts/build-mac-core.sh --archs $(MAC_ARCH) --if-changed
	$(XCODETEST) -only-testing:QueueFocusUITests

release-mac:     ## the macOS release in dist/: universal, Developer ID, notarized, in a DMG
	scripts/release-mac.sh

release-mac-dry-run: ## the same signed ad hoc and not notarized, to check it all without the certificate
	scripts/release-mac.sh --dry-run

version: set-version  ## set VERSION everywhere; prompts when VERSION is omitted

set-version:
	@if [ -n "$${REQUESTED_VERSION:-}" ] && [ -n "$${REQUESTED_EXTENSION_VERSION:-}" ]; then \
		scripts/set-version "$$REQUESTED_VERSION" --extension-version "$$REQUESTED_EXTENSION_VERSION"; \
	elif [ -n "$${REQUESTED_VERSION:-}" ]; then \
		scripts/set-version "$$REQUESTED_VERSION"; \
	elif [ -n "$${REQUESTED_EXTENSION_VERSION:-}" ]; then \
		scripts/set-version --interactive --extension-version "$$REQUESTED_EXTENSION_VERSION"; \
	else \
		scripts/set-version --interactive; \
	fi

maybe-version:
	@$(MAKE) --no-print-directory set-version

install: maybe-version  ## prompt for VERSION, then install the latest working tree
	scripts/install-local.sh

uninstall:       ## remove the per-user install (keeps your tasks)
	scripts/install-local.sh --uninstall

update:          ## pull latest code, prompt for VERSION, and reinstall
	@if git remote get-url origin >/dev/null 2>&1; then git pull --ff-only; else echo "no git remote; installing the working tree"; fi
	$(MAKE) install

deb: $(SCHEMAS)/gschemas.compiled     ## build target/debian/queue-focus_*.deb (needs cargo-deb)
	$(CARGO) deb -p queue-focus

run: build       ## run the service in the foreground (debug)
	target/release/queue-focus service

clean:
	$(CARGO) clean
	rm -f $(SCHEMAS)/gschemas.compiled
