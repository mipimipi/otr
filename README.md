[![REUSE status](https://api.reuse.software/badge/gitlab.com/mipimipi/otr)](https://api.reuse.software/info/gitlab.com/mipimipi/otr)
[![Crates.io](https://img.shields.io/crates/v/otr.svg)](https://crates.io/crates/otr)

# otr

otr is a command line tool that decodes and cuts video files from [Online TV Recorder](https://www.onlinetvrecorder.com/) (OTR). It is running on Linux and macOS.

Supported architectures are:

- [x86_64](https://en.wikipedia.org/wiki/X86-64)/amd64
- [AArch64](https://en.wikipedia.org/wiki/AArch64)/arm64, incl. platforms such as [Raspberry Pi 4](https://en.wikipedia.org/wiki/Raspberry_Pi_4) and [Apple Silicon, M series](https://en.wikipedia.org/wiki/Apple_silicon#M_series)

## Features

### Decoding

otr decodes OTRKEY files (i.e., encoded video files downloaded from OTR). The decoding functionality is based on the work of eddy14, who reverse-engineered the OTRKEY file format, see [his blog post](https://pyropeter.eu/41yd.de/blog/2010/04/18/otrkey-breaker/) [German, mirrored by [PyroPeter](https://github.com/pyropeter)].

Decoding includes verifying the checksums of the OTRKEY file and the decoded file.

### Cutting

For cutting videos, there are two different options:

1. otr downloads cut lists from the cut list provider [cutlist.at](http://cutlist.at) automatically
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

### Manual installation

This works for both, Linux and macOS. Make sure to install MKVToolNix, since - as already mentioned - otr requires MKVmerge for cutting videos.

To download otr, clone this repository via

    git clone https://gitlab.com/mipimipi/otr.git

After that, build otr by executing

    cd otr
    make

Finally, execute

    make install

as `root` to install otr.

### Installation with package managers

For Arch Linux (and other Linux distros, that can install packages from the Arch User Repository) there is a [otr package in AUR](https://aur.archlinux.org/packages/otr-git/).

### OTRKEY mimetype

On Linux, to create a dedicated mimetype for OTRKEY files and to make otr the default application for that type, the `resources` folder of this repository contains two files. Just copy them to the corresponding folders of your machine:

	cp resources/otr.desktop /usr/share/applications/.
	cp resources/otrkey_mime.xml /usr/share/mime/packages/.

Since otr is then the only application that can process files of the new mime type, it should now be called automatically if you double click on an otrkey file.

## Usage

`otr process` processes all video files that are submitted as command line parameters and the files stored in the working directory (see below). `otr decode`, `otr cut` allow decoding / cutting a single video. See the command line help for details.

otr requires a certain schema for the name of video files (that is the schema OTR uses as well). See schema in pseudo regular expression notation for encoded and decoded files:

    <name-of-video>_YY.MM.DD_hh-mm_<TV-station>_<a-number>_TVOON_DE.mpg(.|.HQ|.HD).<format>(.otrkey)?

Since MKVmerge is used to cut videos, the resulting files have the [Matroska container format](https://en.wikipedia.org/wiki/Matroska) (.mkv). To be able to play such videos on macOS with [Quicktime](https://support.apple.com/guide/quicktime-player/welcome/mac) a plugin is required. Otherwise, a different player must be used - [VLC](https://www.videolan.org/vlc) for example.

### Configuration

otr requires just 3 pieces of information:

* the working directory (see below)
* your OTR user
* your OTR password

These parameters can either be submitted on the command line or (more convenient) stored in a [JSON](https://en.wikipedia.org/wiki/JSON) configuration file. The path of this file can also either be submitted on the command line when otr is called or (again more convenient), the default configuration path of your operation system is used. On Linux, that is `<XDG-CONFIG-HOME-DIR>/otr.json`, whereas in most cases `<XDG-CONFIG-HOME-DIR>` equals to `~/.config`. On macOS, that is `~/Library/Application Support/otr.json`.

The configuration file could look like this:

	{
		"user": "<YOUR OTR USER",
		"password": "<YOUR OTR PASSWORD>",
		"working_dir": "<PATH TO YOUR OTR WORKING DIRECTORY>"
	}

### Working Directory

otr requires a working directory (for example, `~/Videos/OTR` on Linux, or `~/Movies/OTR`on macOS). In this directory, the sub directories `Encoded`, `Decoded` and `Cut` are created. Thus, the directory structure is like so:

    <otr working dir>
        |
        |- Encoded
        |
        |- Decoded
        |   |- Archive
        |
        |- Cut

There, video files are stored depending on their processing status. I.e., `Cut` contains the video files that have been cut, `Decoded` the decoded files that have not been cut yet (it can happen that a video can be decoded but cannot be cut because cut lists do not exist yet). If videos have been cut, the uncut version is stored under `Decoded/Archive` to allow users to repeat the cutting if they are not happy with the result.

# License

[GNU Public License v3.0](https://gitlab.com/mipimipi/otr/blob/main/LICENSE)
