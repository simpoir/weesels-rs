
.PHONY: coverage
coverage:
	rm -f target/debug/deps/weesels-*
	rm target/cov -rf
	cargo build --tests
	kcov --include-path=src/ target/cov target/debug/deps/weesels-*[0-9a-f][0-9a-f]
	xdg-open target/cov/index.html
