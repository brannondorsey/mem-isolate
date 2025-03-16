# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2](https://github.com/brannondorsey/mem-isolate/compare/v0.1.1...v0.1.2) - 2025-03-16

### Added

- Add `tracing` feature ([#24](https://github.com/brannondorsey/mem-isolate/pull/24))

### Other

- Cut a new release only when the release PR is merged ([#28](https://github.com/brannondorsey/mem-isolate/pull/28))
- Clarify function purity definition in README and module description ([#26](https://github.com/brannondorsey/mem-isolate/pull/26))
- Add a benchmark indicating how fast a fork() could be without a wait() afterwards ([#23](https://github.com/brannondorsey/mem-isolate/pull/23))
- Refactor for readability ([#19](https://github.com/brannondorsey/mem-isolate/pull/19))

## [0.1.1](https://github.com/brannondorsey/mem-isolate/compare/v0.1.0...v0.1.1) - 2025-03-14

### Other

- Split LICENSE into LICENSE-MIT and LICENSE-APACHE ([#16](https://github.com/brannondorsey/mem-isolate/pull/16))
- Pin to release-plz action version for security ([#15](https://github.com/brannondorsey/mem-isolate/pull/15))
- Run clippy with --all-targets via CI ([#14](https://github.com/brannondorsey/mem-isolate/pull/14))
- Enable and conform to more strict clippy lints ([#13](https://github.com/brannondorsey/mem-isolate/pull/13))
