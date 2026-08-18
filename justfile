plugin_id := "herdr-flash"

# list available recipes
default:
    @just --list

# build the release binary
build:
    cargo build --release

# format, lint, and test - run before committing
check: fmt-check clippy test

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

clippy:
    cargo clippy --release -- -D warnings

test:
    cargo test

# register this checkout with a running local Herdr as a linked (dev) plugin
link: build
    herdr plugin link .

# remove the linked dev plugin
unlink:
    herdr plugin unlink {{ plugin_id }}

# rebuild and re-link in one step, e.g. after pulling changes
relink: unlink link

# open the plugin's popup pane directly, bypassing any keybind
open:
    herdr plugin pane open --plugin {{ plugin_id }} --entrypoint flash --placement popup
