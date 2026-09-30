# Contributing to sqv

Thanks for your interest in contributing to `sqv`.

`sqv` is a keyboard-first terminal viewer for SQLite databases. Contributions are welcome, including bug reports, documentation improvements, small fixes, and feature ideas.

## Getting started

1. Fork the repository.
2. Clone your fork:

   ```bash
   git clone https://github.com/YOUR_USERNAME/sqv.git
   cd sqv
   ```

3. Create a new branch:

   ```bash
   git checkout -b your-change-name
   ```

4. Build or run the project:

   ```bash
   cargo run --release -- path/to/database.db
   ```

## Development checks

Before opening a pull request, please run:

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

Please make sure your changes pass these checks.

## User guide

The [published user guide](https://mendrik-private.github.io/sqv/) lives in
`docs/user-guide` and uses mdBook, like the Diorama user guide. Install the pinned
builder and validate the site, including local links, images, and anchors:

```bash
cargo install mdbook --version 0.5.4 --locked
sh build-aux/build-docs.sh
```

The link checker requires Ruby. Build output goes to `target/docs-site`.
For a live local preview:

```bash
mdbook serve docs/user-guide --open
```

Add new chapters to `docs/user-guide/src/SUMMARY.md`. Keep screenshots under
`docs/user-guide/src/assets/screenshots`. The shortcut chapter includes the
README keymap section, which `cargo test` checks against the application's help.
When bindings change, run `SQVIEW_UPDATE_README=1 cargo test` to update those tables.

The **Documentation** workflow builds and checks the guide on pull requests and
pushes to `main`. Successful pushes to `main` deploy to GitHub Pages; manual
deployment is also available through **Run workflow** on `main`. Repository
Settings → Pages must use **GitHub Actions** as the publishing source.

## Reporting bugs

When reporting a bug, include:

- What you were trying to do
- What happened
- What you expected to happen
- Your operating system and terminal
- The `sqv` version or commit you are using
- A small example database or reproduction steps, if possible

## Suggesting features

Feature requests are welcome. Please describe:

- The problem you want to solve
- The behavior you would like
- Any alternatives you considered

## Pull requests

When opening a pull request:

- Keep the change focused
- Explain what changed and why
- Add or update tests when relevant
- Update documentation if user-facing behavior changes

## Code style

Follow the existing Rust style in the project.

Use:

```bash
cargo fmt
```

to format code before submitting.

## License

By contributing to this project, you agree that your contributions will be licensed under the same license as the project.
