# queue-focus — common tasks. `make help` lists them.
CARGO   := scripts/cargo
UUID    := queue-focus@queuefocus.org
SCHEMAS := extension/$(UUID)/schemas

.PHONY: help build test check install uninstall update deb clean run

help:            ## show this help
	@grep -E '^[a-z-]+:.*##' $(MAKEFILE_LIST) | awk -F':.*##' '{printf "  %-10s %s\n", $$1, $$2}'

build: $(SCHEMAS)/gschemas.compiled   ## release build (no install)
	$(CARGO) build --release -p queue-focus

$(SCHEMAS)/gschemas.compiled: $(SCHEMAS)/*.gschema.xml
	glib-compile-schemas --strict $(SCHEMAS)

test:            ## unit tests
	$(CARGO) test --workspace

check:           ## fmt + clippy + extension syntax
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings
	node --check extension/$(UUID)/extension.js

install:         ## build and install for the current user (~/.local), restart the service
	scripts/install-local.sh

uninstall:       ## remove the per-user install (keeps your tasks)
	scripts/install-local.sh --uninstall

update:          ## pull the latest version (if this is a git clone with a remote) and reinstall
	@if git remote get-url origin >/dev/null 2>&1; then git pull --ff-only; else echo "no git remote; installing the working tree"; fi
	$(MAKE) install

deb: $(SCHEMAS)/gschemas.compiled     ## build target/debian/queue-focus_*.deb (needs cargo-deb)
	$(CARGO) deb -p queue-focus

run: build       ## run the service in the foreground (debug)
	target/release/queue-focus service

clean:
	$(CARGO) clean
	rm -f $(SCHEMAS)/gschemas.compiled
