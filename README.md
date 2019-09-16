# SCFS – SplitCatFS

A convenient splitting and concatenating filesystem.

## Motivation

### History

While setting up a cloud based backup and archive solution, I encountered the
following phenomenon: Many small files would get uploaded quite fast and –
depending on the actual cloud storage provider – highly concurrently, while
big files tend to slow down the whole process. The explanation is simple, most
cloud storage providers do not support the upload of a single file, sometimes
they would not even support resuming a partial upload. You would need to
upload it in one go, sequentially byte for byte, it's all or nothing.

Now consider a scenario, where you upload a really big file, like a mirror of
your Raspberry Pi's SD card with the system and configuration on it. I have
such a file, it is about 4 GB big. Now, while backing up my system, this was
the last file to be uploaded. According to ETA calculations, it would have
taken several hours, so I let it run overnight. The next morning I found out
that after around 95% of upload process, my internet connection vanished for
just a few seconds, but long enough that the transfer tool aborted the upload.
The temporary file got deleted from the cloud storage, so I had to start from
zero again. Several hours of uploading wasted.

I thought of a way to split big files, so that I can upload it more
efficiently, but I came to the conclusion, that manually splitting files,
uploading them, and deleting them afterwards locally, is not a very scalable
solution.

So I came up with the idea of a special filesystem. A filesystem that would
present big files as if they were many small chunks in separate files. In
reality, the chunks would all point to the same physical file, only with
different offsets. This way I could upload chunked files in parallel without
losing too much progress, even if the upload gets aborted midway.

*SplitFS* was born.

In the future, I also want to include the reverse variant, which I would call
*CatFS*. If I download such a chunked file, I would `cat * >file` to re-create
the actual file. *CatFS* would do this transparently. It would recognize a
chunked file and present it as a complete file.


### Why Rust?

I am relatively new to Rust and I thought, the best way to deepen my
understanding with Rust is to take on a project that would require dedication
and a certain knowledge of the language.

## Installation

SCFS can be installed easily through Cargo via `crates.io`:

    cargo install scfs

## Usage

To mount a directory with SplitFS, use the following form:

    scfs <base directory> <mount point>

The directory specified as `mount point` will now reflect the content of `base
directory`, replacing each regular file with a directory that contains
enumerated chunks of that file as separate files.

## Limitations

Please be aware that this project is merely a raw prototype for now!
Specifically:

-   It only works on Linux for now, maybe even on UNIX. But definitely not on
    Windows or MacOS.

-   It can only work with directories and regular files. Every other file type
    will be ignored or may end end up in a `panic!`.

-   The base directory will be mounted read-only in the new mount point and it
    is expected that it will not be altered while mounted.

-   *CatFS* is de facto not existent by the time of writing. But do not worry,
    you can simply manually `cat` all the chunks to retrieve the complete
    file.
