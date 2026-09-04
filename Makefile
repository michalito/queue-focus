# queue-focus — common tasks. `make help` lists them.
CARGO   := scripts/cargo
UUID    := queue-focus@queuefocus.org
SCHEMAS := extension/$(UUID)/schemas
# Only explicit command-line values affect version files. Ambient environment
# variables named VERSION must never make a plain `make install` mutate them.
REQUESTED_VERSION           := $(if $(filter command line,$(origin VERSION)),$(VERSION),)
REQUESTED_EXTENSION_VERSION := $(if $(filter command line,$(origin EXTENSION_VERSION)),$(EXTENSION_VERSION),)
export REQUESTED_VERSION REQUESTED_EXTENSION_VERSION

.PHONY: help build test test-install test-version test-extension check version set-version maybe-version install uninstall update deb clean run

help:            ## show this help
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | awk -F':.*##' '{printf "  %-13s %s\n", $$1, $$2}'

build: $(SCHEMAS)/gschemas.compiled   ## release build (no install)
	$(CARGO) build --release -p queue-focus

$(SCHEMAS)/gschemas.compiled: $(SCHEMAS)/*.gschema.xml
	glib-compile-schemas --strict $(SCHEMAS)

test:            ## unit + isolated integration tests
	$(CARGO) test --workspace
	node extension/test/flash.test.mjs
	scripts/test-install-local.sh
	scripts/test-set-version.sh

test-install:    ## isolated local installer integration tests
	scripts/test-install-local.sh

test-version:    ## isolated versioning integration tests
	scripts/test-set-version.sh

test-extension:  ## shell-extension tests, against a stubbed GNOME Shell
	node extension/test/flash.test.mjs

check:           ## fmt + clippy + JS/Python/shell syntax
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	for js in extension/$(UUID)/*.js; do node --check "$$js"; done
	bash -n scripts/*.sh
	python3 -c 'from pathlib import Path; compile(Path("scripts/set-version").read_text(), "scripts/set-version", "exec")'

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
