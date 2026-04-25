#!/bin/bash
BIN=./target/debug/gno-rs
for d in tests/ui/*/; do
    name=$(basename "$d")
    if [ "$name" = "foreign_importing" ]; then
        echo "SKIP: $name"
        continue
    fi
    output=$($BIN run --output-assert "${d}main.go" 2>&1)
    ec=$?
    if [ $ec -eq 0 ]; then
        echo "PASS: $name"
    else
        echo "FAIL: $name"
    fi
done
