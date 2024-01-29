[![Crates.io](https://img.shields.io/crates/v/otr.svg)](https://crates.io/crates/otr)
[![Documentation](https://docs.rs/otr/badge.svg)](https://docs.rs/otr)
[![REUSE status](https://api.reuse.software/badge/gitlab.com/mipimipi/otr)](https://api.reuse.software/info/gitlab.com/mipimipi/otr)

# otr

**Structure of configuration file changed in an incompatible way with v0.8.0! - see details [here](#configuration)**

otr is a command line tool that decodes and cuts video files from [Online TV Recorder](https://www.onlinetvrecorder.com/) (OTR). It is running on Linux, macOS and Windows.

Supported architectures are:

- [x86_64](https://en.wikipedia.org/wiki/X86-64)/amd64
- [AArch64](https://en.wikipedia.org/wiki/AArch64)/arm64, incl. platforms such as [Raspberry Pi 4](https://en.wikipedia.org/wiki/Raspberry_Pi_4) and [Apple Silicon, M series](https://en.wikipedia.org/wiki/Apple_silicon#M_series)

## Table of contents

- [Features](#features)
- [Installation](#installation)
    - [Linux](#linux)
    - [macOS](#macos)
    - [Windows](#windows)
- [Configuration](#configuration)
- [Running otr](#running-otr) 

## Features

### Decoding

otr decodes OTRKEY files (i.e., encoded video files downloaded from OTR). The decoding functionality is based on the work of eddy14, who reverse-engineered the OTRKEY file format, see [his blog post](https://pyropeter.eu/41yd.de/blog/2010/04/18/otrkey-breaker/) [German, mirrored by [PyroPeter](https://github.com/pyropeter)].

Decoding includes verifying the checksums of the OTRKEY file and the decoded file.

### Cutting

For cutting videos, there are two different options:

1. otr downloads cut lists from the cut list provider [cutlist.at](http://cutlist.at) automatically. If multiple cut lists are available, otr prefers those with a high rating.
1. Cut intervals can be specified on the command line

	This option can make sense if cutlist.at cannot provide a cut list for a certain video. In this case, the cut intervals could be determined (manually) with a video editor such as [Avidemux](https://avidemux.sourceforge.net/), [OpenShot](https://www.openshot.org/) or [Shotcut](https://www.shotcut.org/)on Linux,  and fed into otr on the command line.

otr cuts videos by using [MKVmerge](https://mkvtoolnix.download/doc/mkvmerge.html) which is part of the [MKVToolNix](https://mkvtoolnix.download/) program suite.

### Fast, concurrent processing

otr tries to process files as fast as possible. Video files are decoded sequentially (i.e., one by one), but each files is decoded using concurrent threads to leverage the cpu capabilities to full extend. Cutting is done for many files simultaneously via concurrent threads as well.

### Automated handling of otrkey files

It is possible to create a dedicated mime type for otrkey files. otr can be defined as default application for it. This repository contains the required files for Linux.

### Simple usage

Though being a command line application, the usage of otr is quite simple. If, for example, you have downloaded some OTRKEY files from OTR, the command `otr process` processes all files (i.e., they are decoded, cut lists are downloaded and the files are cut). With the dedicated mime type, it is even simpler: A double click on an OTRKEY file starts otr.

## Installation

### Linux

#### Manual installation

This works for both, Linux and macOS. Make sure to install MKVToolNix, since - as already mentioned - otr requires MKVmerge for cutting videos.

To download otr, clone this repository via

    git clone https://gitlab.com/mipimipi/otr.git

After that, build otr by executing

    cd otr
    make

Finally, execute

    make install

as `root` to install otr.

#### Installation with package managers

For Arch Linux (and other Linux distros, that can install packages from the Arch User Repository) there are the AUR packages [otr](https://aur.archlinux.org/packages/otr/) and [otr-git](https://aur.archlinux.org/packages/otr-git/). These packages are also available as binaries via the [nerdstuff repository](https://nerdstuff.org/repository/).

#### OTRKEY mimetype

On Linux, to create a dedicated mimetype for OTRKEY files and to make otr the default application for that type, the `resources` folder of this repository contains two files. Just copy them to the corresponding folders of your machine:

	cp resources/otr.desktop /usr/share/applications/.
	cp resources/otrkey_mime.xml /usr/share/mime/packages/.

Since otr is then the only application that can process files of the new mime type, it should now be called automatically if you double click on an otrkey file.

### macOS

See [Manual installation](#manual-installation). Files that were cut with otr have the [Matroska container format](https://en.wikipedia.org/wiki/Matroska) (.mkv). To be able to play such videos with [Quicktime](https://support.apple.com/guide/quicktime-player/welcome/mac) a plugin is required. Otherwise, a different player must be used - [VLC](https://www.videolan.org/vlc) for example.

### Windows

The installation on Windows is a little bit cumbersome. Since there is no Windows installer for otr, it works purely manual:

1. Make sure that [MKVToolNix](https://mkvtoolnix.download/) is installed and that the path of the binaries (`mkvmerge`, etc.) is contained in the Windows path environment variable (see [here](https://helpdeskgeek.com/windows-10/add-windows-path-environment-variable/) how to achieve this).
1. In a terminal window, clone the otr repository ([git](https://git-scm.com/download/win) must be installed as prerequisite, of course) and switch to the new directory:

		git clone https://gitlab.com/mipimipi/otr.git
		cd otr
		
1. Build the otr binary ([rust](https://forge.rust-lang.org/infra/other-installation-methods.html) must be installed as prerequisite, of course):

		cargo build --release
		   
1. If the build step ran through without errors, the otr binary (`otr.exe`) is stored in the sub directory `target\release`. Copy the binary wherever you want. You might want to add its new path to the Windows path variable as described above.

1. Maintain the otr configuration file. It must be stored in the directory `C:\Users\<YOUR_USER_NAME>\AppData\Roaming`. Replace `<YOUR_USER_NAME>` by your Windows user name.

1. Have fun with otr.

## Configuration

otr can be configured by creating a configuration file in [JSON](https://en.wikipedia.org/wiki/JSON) format. It is named `otr.json` and stored in the default configuration directory of your OS. That is ...

- `<XDG-CONFIG-HOME-DIR>` on Linux, whereas in most cases `<XDG-CONFIG-HOME-DIR>` equals to `~/.config`
- `~/Library/Application Support` on macOS
- `C:\Users\<YOUR_USER_NAME\AppData\Roaming`

The configuration file has this structure:

	{
		"working_dir": "<PATH TO YOUR OTR WORKING DIRECTORY>",
        "decoding": {
    		"user": "<YOUR OTR USER>",
	    	"password": "<YOUR OTR PASSWORD>",
        },
        "cutting": {
            "min_cutlist_rating": <MINIMUM CUT LIST RATING>
        }
	}

All parameters are optional and/or have default values, or can be specified/overwritten by a corresponding command line parameter. This table explains the details:

| Parameter | Description | Mandatory | Default | CLI parameter |
|---|---|---|---|---|
| `working_directory` | [Working directory](#working-directory) of otr | Optional | `~/Videos/OTR` on Linux, `~/Movies/OTR`on macOS, `C:\Users\<YOUR_USER_NAME>\Videos\OTR` on Windows | No |
| `user`, `password`| Access data for Online TV Recorder | Mandatory for decoding videos | There is no default | Yes (`--user/-u` and `--password/-p`)|
| `min_cutlist_rating` | Minimum rating that a cut list from cutlist.at must have to be accepted by otr for cutting videos | Optional | If the parameter is not given, all cut lists are accepted |  Yes (`--min-rating`) |

### Working Directory

otr requires a working directory. In this directory, the sub directories `Encoded`, `Decoded` and `Cut` are created. Thus, the directory structure is like so:

    <otr working dir>
        |
        |- Encoded
        |
        |- Decoded
        |   |- Archive
        |
        |- Cut

There, video files are stored depending on their processing status. I.e., `Cut` contains the video files that have been cut, `Decoded` the decoded files that have not been cut yet (it can happen that a video can be decoded but cannot be cut because cut lists do not exist yet). If videos have been cut, the uncut version is stored under `Decoded/Archive` to allow users to repeat the cutting if they are not happy with the result.

## Running otr

### `otr process`

`otr process` processes all video files that are either submitted as command line parameters, or stored in the [working directory](#working-directory).

otr requires a certain schema for the name of video files (that is the schema OTR uses as well). See schema in pseudo regular expression notation for encoded and decoded files:

    <name-of-video>_YY.MM.DD_hh-mm_<TV-station>_<a-number>_TVOON_DE.mpg(.|.HQ|.HD).<format>(.otrkey)?

Since MKVmerge is used to cut videos, the resulting files have the [Matroska container format](https://en.wikipedia.org/wiki/Matroska) (.mkv). 

### `otr decode`

 `otr decode` allows decoding a single video. See the command line help for details.

### `otr cut`

 `otr cut` allows cutting a single video. The cut list that is used for that can either be selected and downloaded automatically from cutlist.at, or submitted via commandline parameters (either as file or as dedicated cut intervals) - see the command line help for details.

## Verbosity
 
The command line flag `--verbose/-v` defines how detailed the message output of otr is. With `--quiet/-q`, there are no messages, See command line help for further details.

## License

[GNU Public License v3.0](https://gitlab.com/mipimipi/otr/blob/main/LICENSE)
