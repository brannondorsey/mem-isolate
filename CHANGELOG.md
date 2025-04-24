# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.6](https://github.com/brannondorsey/mem-isolate/compare/v0.1.5...v0.1.6) - 2025-04-24

### Added

- *(tests)* Run tests on MacOS in addition to Ubuntu ([#60](https://github.com/brannondorsey/mem-isolate/pull/60))

### Fixed

- Ensure large results are handled correctly ([#56](https://github.com/brannondorsey/mem-isolate/pull/56))

### Other

- Add empty test and TODO comment ([#59](https://github.com/brannondorsey/mem-isolate/pull/59))
- Add test that covers the case when the user-defined callable panics ([#57](https://github.com/brannondorsey/mem-isolate/pull/57))

## [0.1.5](https://github.com/brannondorsey/mem-isolate/compare/v0.1.4...v0.1.5) - 2025-04-20

### Fixed

- Run tests with one thread only as a fix for flaky tests that hang indefinitely ([#49](https://github.com/brannondorsey/mem-isolate/pull/49)). See [#52](https://github.com/brannondorsey/mem-isolate/pull/52) for how we tested this fix.

### Other

- Add 1 second default timeouts to all tests ([#50](https://github.com/brannondorsey/mem-isolate/pull/50))
- Bump test timeouts for two flaky tests ([#53](https://github.com/brannondorsey/mem-isolate/pull/53))
- Error on clippy warnings and check formatting in CI ([#48](https://github.com/brannondorsey/mem-isolate/pull/48))

## [0.1.4](https://github.com/brannondorsey/mem-isolate/compare/v0.1.3...v0.1.4) - 2025-04-15

### Other

- Include discussion section in README with links to Hacker News and Reddit posts ([#47](https://github.com/brannondorsey/mem-isolate/pull/47))
- Clarify safety claims and stress limitations in README and module documentation ([#44](https://github.com/brannondorsey/mem-isolate/pull/44))
- Add examples illustrating how to block and restore signals around mem-isolate calls ([#45](https://github.com/brannondorsey/mem-isolate/pull/45))

## [0.1.3](https://github.com/brannondorsey/mem-isolate/compare/v0.1.2...v0.1.3) - 2025-04-05

### Fixed

- Handle syscall interruption by retrying `EINTR` errors encountered during `waitpid()` ([#35](https://github.com/brannondorsey/mem-isolate/pull/35))

### Other

- Cargo update ([#39](https://github.com/brannondorsey/mem-isolate/pull/39))
- Simplify MockableSystemFunctions Debug impl ([#37](https://github.com/brannondorsey/mem-isolate/pull/37))
- Add optional tracing to tests ([#36](https://github.com/brannondorsey/mem-isolate/pull/36))
- Refactor `fork()` into a subroutine ([#29](https://github.com/brannondorsey/mem-isolate/pull/29))

## [0.1.2](https://github.com/brannondorsey/mem-isolate/compare/v0.1.1...v0.1.2) - 2025-03-16

### Added

- Add `tracing` feature ([#24](https://github.com/brannondorsey/mem-isolate/pull/24))

### Other

- Cut a new release only when the release PR is merged ([#28](https://github.com/brannondorsey/mem-isolate/pull/28))
- Clarify function purity definition in README and module description ([#26](https://github.com/brannondorsey/mem-isolate/pull/26))
- Add a benchmark indicating how fast a `fork()` could be without a `wait()` afterwards ([#23](https://github.com/brannondorsey/mem-isolate/pull/23))
- Use `cargo-hack` in CI to test all feature combinations ([#25](https://github.com/brannondorsey/mem-isolate/pull/25))
- Refactor for readability ([#19](https://github.com/brannondorsey/mem-isolate/pull/19))

## [0.1.1](https://github.com/brannondorsey/mem-isolate/compare/v0.1.0...v0.1.1) - 2025-03-14

### Other

- Split LICENSE into LICENSE-MIT and LICENSE-APACHE ([#16](https://github.com/brannondorsey/mem-isolate/pull/16))
- Pin to release-plz action version for security ([#15](https://github.com/brannondorsey/mem-isolate/pull/15))
- Run clippy with --all-targets via CI ([#14](https://github.com/brannondorsey/mem-isolate/pull/14))
- Enable and conform to more strict clippy lints ([#13](https://github.com/brannondorsey/mem-isolate/pull/13))
