
## Show this help
.PHONY: help
help:
	@echo 'Targets:'
	@awk 'match($$0, /^## (.*)$$/, a) {doc=a[1]} /^\w+:/ {if (doc) {printf "%12s %s\n", $$1, doc; doc=""}}' $(lastword $(MAKEFILE_LIST)) | sort

.PHONY: dev_deps
dev_deps:
	@which cargo-add >/dev/null || cargo install cargo-edit
	@which cargo-sort-ck >/dev/null || cargo install cargo-sort-ck

.PHONY: coverage_run
coverage_run: dev_deps
	rm -f target/debug/deps/weesels-*
	rm target/cov -rf
	cargo build --tests
	kcov --include-path=src/ target/cov target/debug/deps/weesels-*[0-9a-f][0-9a-f]

## Run kcov coverage and load report.
.PHONY: coverage
coverage: coverage_run
	xdg-open target/cov/index.html


## Check style and warnings.
.PHONY: lint
lint: dev_deps
	cargo sort-ck
	cargo clippy

## Run all checks.
.PHONY: check
check: lint coverage

## Format source code.
.PHONY: format
format:
	cargo sort-ck -w
	cargo fmt

## Remove build artifacts
.PHONY: clean
clean:
	rm -rf target
