# Lemons Search WASM

Rust/WASM fuzzy search core for Lemons Search.

This repository builds the `@lemons_dev/lemons-search` web-target WASM package
so the main Obsidian plugin can consume prebuilt release artifacts instead of
building Rust code during every plugin build.

## Commands

```sh
make build
make test
make fmt-check
make lint
```

`cargo test` is not the right test command here because the crate defaults to
the `wasm32-unknown-unknown` target. Use `make test`, which runs
`wasm-pack test --node`.

## Release

Before tagging a release, update the package version in `Cargo.toml`. That
version is used by `wasm-pack` when it generates `pkg/package.json`.

Push a `v*` tag to build, test, pack, and publish a GitHub release tarball:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The generated `pkg/` directory is ignored locally. Release artifacts are built
fresh in GitHub Actions.
