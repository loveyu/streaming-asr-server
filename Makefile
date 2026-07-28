.PHONY: dev test

dev:
	cargo run

test:
	cargo test -- --nocapture
