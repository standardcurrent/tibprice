#! /bin/bash
export CROSS_CONTAINER_OPTS="--platform=linux/amd64"
cross build --target armv7-unknown-linux-gnueabihf --release
