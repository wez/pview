
check:
	cargo check

fmt:
	cargo +nightly fmt

addon:
	docker run \
		--rm \
		--privileged \
		-v ./addon:/data \
			ghcr.io/home-assistant/amd64-builder:latest \
			--all \
			--test \
			--target /data

.PHONY: addon fmt check
