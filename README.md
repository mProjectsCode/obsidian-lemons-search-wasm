# Lemons Search WASM

Rust/WASM fuzzy search core for Lemons Search.

This repository builds the `@lemons_dev/lemons-search` web-target WASM package
so the main Obsidian plugin can consume prebuilt release artifacts instead of
building Rust code during every plugin build.

## Commands

```sh
npm run build
npm run test
npm run fmt:check
npm run lint
```

`cargo test` is not the right test command here because the crate defaults to
the `wasm32-unknown-unknown` target. Use `npm run test`, which runs
`wasm-pack test --node`.

## Release

Push a `v*` tag to build, test, pack, and publish a GitHub release tarball:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The generated `pkg/` directory is ignored locally. Release artifacts are built
fresh in GitHub Actions.
