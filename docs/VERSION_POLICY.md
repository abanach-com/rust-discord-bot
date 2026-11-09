# Version policy

ZuckerBot tracks current stable releases rather than intentionally remaining on old major versions.

## Rules

1. `rust-toolchain.toml` pins the exact latest stable Rust patch release.
2. `Cargo.toml` pins every direct Rust dependency to the exact stable version verified for this repository.
3. GitHub Actions use the newest supported major release and are watched by Dependabot.
4. The production image uses the current stable Debian release and an exact Rust image tag.
5. PostgreSQL uses the newest production-ready major and patch release; beta, release-candidate and nightly builds are forbidden in production.
6. `yt-dlp` is downloaded from its immutable upstream release and verified with SHA-256.
7. OS packages come from the current stable Debian security repositories. “Latest” never means an unreviewed nightly build.
8. A version bump is not complete until formatting, Clippy, tests, release build and container build all pass.
9. Dependency changes must preserve the Rust/Lua trust boundary and must not expose network, process, filesystem, database or Discord clients to Lua.
10. When an upstream release is incompatible, the repository opens a migration task immediately; it must not silently downgrade.

## Automated enforcement

- `scripts/check-latest-rust.sh` compares the pinned toolchain with Rust's official stable channel manifest during CI.
- `scripts/check-branch-protection.py` performs a read-only smoke check on `main` branch protection settings.
- Dependabot checks Cargo, GitHub Actions and Docker dependencies every day.
- CI resolves the pinned dependency graph, runs Clippy and tests, builds the release binary and builds the production container.
- Concurrency in CI cancels redundant runs only for non-main refs (`cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}`), preventing cancellation of `main` branch builds.

## Branch protection and quality gates

To enforce quality gates before merging into `main`, repository settings should configure:
1. **Branch Protection Rule / Ruleset for `main`**:
   - `Require status checks to pass before merging` enabled.
   - Required checks: `Format, lint, test and build`, `Build production container`, `Security and supply-chain gates`.
   - `Require branches to be up to date before merging`.
   - `Block force pushes` and `Block branch deletion`.
   - Prevent administrative bypass where feasible.
2. Note: Modifying GitHub repository branch rules requires administrative privileges on `abanach-com/rust-discord-bot` (Settings -> Branches / Rulesets). Token with standard write permissions cannot modify branch protection directly.

## Baseline verified on 2026-09-02

| Component | Version/channel |
| --- | --- |
| Rust | 1.99.1 |
| Rust edition | 2024 |
| Discord API | v10 |
| Serenity | 0.12.5 |
| Songbird | 0.6.0 |
| mlua | 0.12.1 |
| Axum | 0.8.9 |
| SQLx | 0.9.0 |
| Tokio | 1.53.1 |
| Reqwest | 0.13.4 |
| PostgreSQL | 18.6 |
| Debian | 13 “trixie” |
| yt-dlp | 2026.08.19 |

This table records the audited baseline, not a permanent ceiling. The automated checks and dependency pull requests supersede it when new stable releases appear.
