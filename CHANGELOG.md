# Changelog

## [Release 0.7.0](https://gitlab.com/mipimipi/otr/tags/v0.7.0) (2024-01-10

### Added

- Support for macOS

## [Release 0.6.0](https://gitlab.com/mipimipi/otr/tags/v0.6.0) (2023-04-07

### Added

- Possibility to read a cut list from file: `otr cut` got a corresponding option
- If cut list intervals are defined based on frame numbers AND time, consider both options. So far, in this case only the fram numbers intervals were used

### Changed

- Syntax of intervals string of `otr cut`: `times` -> `time`

## [Release 0.5.0](https://gitlab.com/mipimipi/otr/tags/v0.5.0) (2023-04-01

### Added

- Possibility to cut single videos by specifying the cut intervals on the command line

### Changed

- Introduced sub commands `process` and `cut`

## [Release 0.4.0](https://gitlab.com/mipimipi/otr/tags/v0.4.0) (2022-09-24

### Changed

- Refactoring: made the code "rustier"

## [Release 0.3.0](https://gitlab.com/mipimipi/otr/tags/v0.3.0) (2022-09-13

### Changed

- Behavior when videos are submitted via cli: only these videos are processed, other videos are ignored
- Migrated from [structopt](https://github.com/TeXitoi/structopt) to [clap](https://docs.rs/clap/latest/clap/)
- Refactoring

## [Release 0.2.0](https://gitlab.com/mipimipi/otr/tags/v0.2.0) (2022-09-09)

### Changed

- Do not tread non-existence of cut lists as error when displaying cut result
