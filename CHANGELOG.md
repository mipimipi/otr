# Changelog

## [Release 0.10.0](https://gitlab.com/mipimipi/otr/tags/v0.10.0) (2024-05-01)

### Changed

- Cutting is done accurate to frames (i.e., even if a boundary of a cut interval is not at a key frame, the videos is cut exactly at that boundary). FFmpeg replaces MKVmerge as toll used for cutting
- Syntax of cut list interval for option `otr cut --cutlist` slightly changed

### Removed

- Description about how to install otr on Windows. This was quite cumbersome and thus most likely nobody did ever do this

## [Release 0.9.0](https://gitlab.com/mipimipi/otr/tags/v0.9.0) (2024-02-09)

### Added

- Automatic generation of cut list files for self-created cut lists and submission of such files to cutlist.at
- New CLI parameter `--rating` to overwrite the default rating for self-created cut lists that can be maintained in the configuration file
- Dedicated check if mkvmerge is installed

## [Release 0.8.0](https://gitlab.com/mipimipi/otr/tags/v0.8.0) (2024-01-29)

### Added

- CLI parameters for verbosity and switching off output completely
- Possibility to define minimum cut list rating - either via CLI parameter or via configuration option
- Dedicated (sub) command for decoding videos (without cutting them)
- Possibility to submit a cut list ID to the cut command (new parameter `--cutlist-id` was introduced for that)
- Verified that otr is running on Windows

### Changed

- otr only displays error messages per default. If more detailed messages are wanted, this must be specified via the new CLI parameter
- Cut list-relevant parameters of cut command unified. `--list/-l` and `--file/-f` were replaced by `--cutlist` and `--cutlist-file`
- Structure of configuration changed incompatibly: Introduced sub structures for decoding and cutting

### Removed

- CLI parameter for configuration file
- CLI parameter for working directory


## [Release 0.7.0](https://gitlab.com/mipimipi/otr/tags/v0.7.0) (2024-01-10)

### Added

- Support for macOS

## [Release 0.6.0](https://gitlab.com/mipimipi/otr/tags/v0.6.0) (2023-04-07)

### Added

- Possibility to read a cut list from file: `otr cut` got a corresponding option
- If cut list intervals are defined based on frame numbers AND time, consider both options. So far, in this case only the frame numbers intervals were used

### Changed

- Syntax of intervals string of `otr cut`: `times` -> `time`

## [Release 0.5.0](https://gitlab.com/mipimipi/otr/tags/v0.5.0) (2023-04-01)

### Added

- Possibility to cut single videos by specifying the cut intervals on the command line

### Changed

- Introduced sub commands `process` and `cut`

## [Release 0.4.0](https://gitlab.com/mipimipi/otr/tags/v0.4.0) (2022-09-24)

### Changed

- Refactoring: made the code "rustier"

## [Release 0.3.0](https://gitlab.com/mipimipi/otr/tags/v0.3.0) (2022-09-13)

### Changed

- Behavior when videos are submitted via cli: only these videos are processed, other videos are ignored
- Migrated from [structopt](https://github.com/TeXitoi/structopt) to [clap](https://docs.rs/clap/latest/clap/)
- Refactoring

## [Release 0.2.0](https://gitlab.com/mipimipi/otr/tags/v0.2.0) (2022-09-09)

### Changed

- Do not tread non-existence of cut lists as error when displaying cut result
