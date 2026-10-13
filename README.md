# Tidewatch

A native macOS app for running AI coding agents, one repo at a time. Each tab
is a fresh git worktree with an agent in it. The terminal sits on top, the
history of that worktree below it, and the diff of the selected commit on the
right. Built by [HoboWare](https://github.com/Hobo-Ware) from the parts of
[Kelp](https://github.com/Hobo-Ware/kelp) (graph, diff, review) and
[stdusk](https://github.com/Hobo-Ware/stdusk) (terminal, tabs, quake window).

Status: in development, private. The release is packaged but nothing is published yet.

## Install

- macOS (once public): `brew install hobo-ware/tap/tidewatch`
- Linux (once public): the `.tar.gz` or the `.AppImage` from the GitHub Release

## Build from source

```sh
cargo run -p tidewatch -- [path]      # run
scripts/bundle-macos.sh               # universal Tidewatch.app in target/dist
scripts/package-linux.sh              # x86_64 Linux tarball and AppImage in target/dist
```

Release steps: [packaging/README.md](packaging/README.md).

- Concept and mockup: [docs/concept.html](docs/concept.html)
- Plan: [PLAN.md](PLAN.md)
- Progress: [LEDGER.md](LEDGER.md)
