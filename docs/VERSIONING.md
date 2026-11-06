# RUX Versioning Policy

## Version identities

- **Compiler/CLI/runtime**: SemVer `MAJOR.MINOR.PATCH[-PRERELEASE]`; the root `VERSION` is the intended next release version until a tagged release is published.
- **Language specification**: `MAJOR.MINOR`, recorded in `LANGUAGE_VERSION`. This is separate from the compiler version.
- **Standard library**: versioned together with the compiler during the 0.x phase. Split versioning may be introduced later with compatibility policy.
- **Documentation/site**: versioned by the language specification; deployment commits do not imply compiler releases.

## Release lifecycle

`alpha.N` -> `beta.N` -> `rc.N` -> stable. A release exists only when a matching Git tag and GitHub Release are published after tests pass. A file named VERSION is not a release.

## SemVer rules

- Before 1.0, breaking changes are permitted between minor releases and must be documented.
- After 1.0, MAJOR means breaking public API or language compatibility changes; MINOR means backward-compatible features; PATCH means backward-compatible fixes.
- Each release must include its supported language specification, target platforms, migration notes, and test status.
- Compiler and runtime should report their actual built version through `rux --version` once the CLI exists.

## Branching

- `main`: reviewed, stable repository state; tagged releases originate here.
- `develop`: integration of upcoming work.
- `feature/*`: isolated features merged through pull requests.
- `release/*`: optional stabilization branches for release candidates.
- `hotfix/*`: urgent fixes to released versions.

## Git tags

Use annotated tags `v0.1.0-alpha.1`, `v0.1.0-beta.1`, `v0.1.0-rc.1`, `v0.1.0`.

## Release gates

1. Parser and type-system conformance tests for implemented features pass.
2. Security and regression tests pass for applicable components.
3. Supported platforms are built and tested; unsupported platforms are explicitly listed.
4. Changelog, compatibility notes and installation instructions are updated.
5. Artifacts and checksums are attached to GitHub Releases when available.

## Proposed roadmap (not released)

| Milestone | Intended scope |
| --- | --- |
| 0.1.0-alpha.1 | Lexer, parser, initial tests |
| 0.2.0-alpha.1 | Types, functions, loop and when |
| 0.3.0-alpha.1 | Native code generation |
| 0.4.0-alpha.1 | Standard library and memory safety |
| 0.5.0-beta.1 | HTTP, API and PostgreSQL |
| 0.6.0-beta.1 | Visual Engine and RUX-authored website |
| 0.7.0-beta.1 | NoSQL and concurrency |
| 0.8.0-rc.1 | Tooling and integration tests |
| 1.0.0 | First stable release after compatibility and security review |

Milestones may change based on implementation and measured quality.
