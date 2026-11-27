run:
	cargo run

test:
	cargo test

# Needs a running, signed-in deskconn daemon.
test-daemon:
	cargo test -- --ignored

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

release:
	cargo build --release

clean:
	cargo clean

.PHONY: run test test-daemon check release clean
