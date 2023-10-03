# Changes since latest release

-   Give proper binary names to sub commands

-   Add cli integration tests

-   Update edition

# Changes in 0.10.1

-   Upgrade fuser to get rid of abandoned users dep

-   Update dependencies

# Changes in 0.10.0

-   Upgrade to clap4

-   Rework and simplify Cli

    We now use the clap derive module to simplify the Cli. Also, split and
    cat are now proper subcommands. Having them introduced as a non-optional
    flag before was a poor decision.

-   Update dependencies

# Changes in 0.9.2

-   Update dependencies to get security fixes

-   Avoid using deprecated fucntion mount_spawn

# Changes in 0.9.1

Update dependencies to fix security issues. Furthermore:

-   Replace dependency fuse with newer fuser

    `fuser` (https://github.com/cberner/fuser) is a more maintained and
    up-to-date fork of `fuse` (https://github.com/zargony/fuse-rs), which
    ensures smoother future development.

-   Use running integer as file handle

    This way, we do not need to rely on the time package to give a pseudo
    unique value.

-   Use running integer as inode

    This way, we do not need to rely on the time package to give a pseudo
    unique value.

-   Remove unused time dependency

# Changes in 0.9.0

-   Check mirror and mountpoint for sanity

    -   Mirror and mountpoint have to exist.

    -   Mirror must not be in a subfolder of mountpoint, to avoid recursive
       mounts. This is also in accordance to EncFS.

-   Run scfs by default to easen rapid development

    Since we build more than just one binary, `cargo run` does not know
    which one to call by default. For this case, there is the key
    "default-run", which tells `cargo run` which binary to use when no
    `--bin` flag is present.

-   Notify main loop when filesystem is dropped

    The filesystem implements the Drop trait now, which makes it possible to
    run a function when the filesystem is unmouted in a way other than by
    terminating the main loop (most prominently by using `umount` directly).

    The previous situation was, when the filesystem was unmounted via
    `umount`, then the main loop would hang infinitely, because there was no
    way to notify the main loop. Now we send a quit signal when the
    filesystem is dropped, so the main loop can exit normally.

-   Canonicalize paths

    Using absolute paths is necessary for a daemon, since a daemon usually
    changes its working directory to "/" so as to not lock a directory.

-   Add daemon flag which puts program in background

    The daemonizing is done after the filesystem has been created, to let
    the initialization happen in foreground. This minimizes the time the
    daemon is running but the filesystem is not mounted yet.

-   Add flag to create mountpoint directory

    The mirror will intentionally *not* be created, since the mount is
    readonly and a missing mirror directory is most likely a typo from the
    user.

-   Add converter for symbolic quantities

    This converter will be used to calculate the blocksize for a SplitFS
    mount. The size can now be given as an integer or optionally with a
    quantifier like "K", "M", "G", and "T", each one multiplying the base
    with 1024.

# Changes in 0.8.0

-   Implement readlink

-   Correctly handle symlinks

-   Replace each metadata with symlink_metadata

    Symlinks should be presented as-is, so it should never be necessary to
    traverse them.

-   Silently ignore unsupported filetypes

-   Add convenience wrappers for catfs and splitfs

    With these wrappers, it is possible to mount the respective filesystem
    without explicitly specifying the mode parameter.

# Changes in 0.7.0

-   Make blocksize customizable

    It is now possible to use a custom blocksize in SplitFS. For example, to
    use 1MB chunks instead of the default size of 2MB, you would go with:

        scfs --mode=split --blocksize=1048576 <base directory> <mount point>

    Where 1048576 is 1024 * 1024, so one megabyte in bytes.

-   Short circuit when reading a size of 0

-   Do not materialize vector after each chunk

    This step was highly unnecessary anyway and it needlessly consumed time
    and memory. An Iterator can be flattened in the same way, but without
    the penalty that comes with materializing.

-   Do not calculate size of last chunk to read

    By simply reading `blocksize` bytes and only taking `size` before
    materializing, we can save a lot of possible mis-calculation regarding
    the last chunk.

    We make use of two properties here:

    -   Reading after EOF is a no-op, so using a higher number on the
        reading operation does not hurt.

    -   The reading operations take only place once we materialize the byte
        array. So even if we issue to read much more bytes than necessary on
        the last chunk, it will not hurt, since we only `take` the correct
        number of bytes on materializing.

-   Fix off-by-one error

-   Correctly handle empty files

    Create at least one chunk, even if it is empty. This way, we can
    differentiate between an empty file and an empty directory.

-   Add test suites to modules

    With automated tests we now can effectively check if new features work
    as intended and that they do not break existing code.

# Changes in 0.6.1

-   Fix misleading part in the README
    
    The misleading part in the README said, that most cloud storage
    providers do not support the upload of a single file. This is of course
    rubbish. What I meant to say was, that they do not support the
    concurrent upload of a single file, as in chunked upload.
    
    This part is fixed now.

-   Update README to reflect CatFS precondition
    
    CatFS will now refuse to mount a directory that was not generated by
    SplitFS prior. The README didn't reflect this breaking change.
    
    This part is fixed now.

# Changes in 0.6.0

-   Remove thread limit
    
    I no longer enforce a thread limit via a thread pool. If you want to
    fire up a thousand threads, then go ahead.

-   Use Option in metadata converter
    
    Using an Option is a more natural way of expressing the intention. If I
    give a ino, use it, if I don't then do something to generate one. At the
    moment that means to take the ino of the existing file in SplitFS or use
    the current timestamp in CatFS.
    
    The Option object is preferred over conventions like using a special
    value such as 0.

-   Use constants for special inos
    
    By using named constants it will be easier to understand what the
    numbers really mean.

-   Calculate additional offset instead of hardcode
    
    If virtual entries like . and .. are added to the directory list, the
    offset needs to be adjusted accordingly. To provide a scalable solution,
    calculate this additional offset instead of just hardcode a specific
    number.

-   Make virtual offset code more readable
    
    To explicitly show when the offset has to be adjusted, I added another,
    yet redundant, conditional block. It is not strictly necessary but it
    makes it possible of adding more such virtual rules in the future
    without adjusting existing code.

-   Add Config struct
    
    This struct will contain possible changeable configuration parameters in
    the future. For now it is empty.

-   Use clap to parse cli arguments

-   Add file name to database
    
    To provide more efficient queries, use the file name in addition to the
    complete path in the database.

-   Use file name in query
    
    By using the file name in the SELECT query instead of iterating over all
    items, the lookups are handled in a much shorter time. This way, even
    directories with a huge amount of files can be listed in reasonable
    time.

-   Create index on parent_ino and file_name
    
    This results in faster queries in lookups.

-   Short circuit readdir
    
    When the readdir buffer is full, the iteration can be suspended. It will
    be resumed by the next filesystem call with the appropriate offset.
    
    This results in a performance boost by not needlessly iterating over
    entries that will be ignored anyway.

-   Remove DISTINCT from SELECT statement
    
    DISTINCT is not necessary in this case and only increases the time
    needed to complete the query.

-   Let FileInfo derive from Default
    
    This way it is possible to easily create a default instance with default
    values for each member.

-   Use converters for correct DB representation
    
    By going the extra mile via the FileInfo-FileInfoRow converter, the
    correct representation of the members in the database can be ensured,
    even with more complex conversion methods, like encoding with JSON or
    the like.

-   Use u8 Vector to represent paths in the db
    
    The conversion to String is lossy and can result in problems with
    special characters that can not be correctly represented. By using a
    byte Vector the paths can be stored raw, without any encoding.

# Changes in 0.5.0

-   Use timestamp as filehandle key
    
    This is an addition to commit 9468cdd884c631e450d6fdaa506e59f1bd2a77e3.
    The described race condition is now also fixed in CatFS.

-   Add threadpool as dependency

-   Cache filenames instead of file handles
    
    By opening the files only when actually called I can avoid race
    conditions that would arise if multiple threads access the same file
    handle.

-   Read files in separate thread
    
    This way, the filesystem is no longer blocked until each portion of a
    file has been read. This also ensures that the kernel may decide to read
    portions of a file in parallel.

-   Include . and .. in directory listing

-   Use timestamp as inode numbers
    
    This way I can save the quite expensive calls to the database for each
    new inode number and hence decreasing the time needed for the initial
    population of the database.

-   Split lib into module files

# Changes in 0.4.0

## Breaking changes

-   From now on, a --mode flag has to be given as first parameter when
    mounting, with either --mode=cat or --mount=split

## Chronologically

-   Add ctrlc as dependency

-   Unmount automatically on SIGINT

-   Use i64 instead of JSON Strings
    
    Converting to JSON is inefficient and prevents proper database
    operations. Using i64 to store a u64 in the database is a way better
    approach. Via the From-trait I can transparently keep using the
    exisiting methods.

-   Use String conversion instead of JSON Strings
    
    Converting OsStrings to Strings is much more efficient than encoding
    them to JSON Strings. Also, JSON Strings in the database prevent proper
    usage of certain database operations, like searching for substrings.

-   Remove serde dependency
    
    Without the need to use JSON Strings, I can get rid of the serde
    dependency.

-   Move populate into SplitFS
    
    Since CatFS will use its own version of populate, it makes sense to put
    the methods to their respective struct implementations.

-   Use termination feature of ctrlc
    
    With this feature enabled, the unmounting will happen not only when a
    SIGINT is caught, but also on SIGTERM.

-   Change return type to Self
    
    This makes it easier to adjust the code for different implementations.

-   Add vdir parameter
    
    This parameter will denote if a directory is just a "virtual directory",
    actually referencing parts of a regular file.

-   Move identical code block to dedicated method

-   Remove offset field from FileHandle
    
    Seeking to a byte position is a neglectable operation, not worth the
    hassle of maintaining an offset field.

-   Implement CatFS
    
    CatFS is the reverse operation to SplitFS. It presents the file parts,
    as displayed by SplitFS as single files again and handles file access to
    the parts transparently.

-   Add mount flag to use CatFS or SplitFS
    
    When mounting a directory, the user now has to give a mode flag.
    
    If the want to use SplitFS, they will need:
    
        scfs --mode=split <base directory> <mount point>
    
    If they want to use CatFS:
    
        scfs --mode=cat <base directory> <mount point>

-   Use timestamp as filehandle key
    
    The previous implementation used the latest created key plus one. This
    leads to a race condition on parallel file access. By using the current
    timestamp in nanoseconds, it is nearly impossible to assign the same key
    to two separate filehandles.

-   Increase TTL to 24 hours
    
    Since the base directory will be mounted read-only and is expected to
    never change during mounting time (at least for now), it is quite
    senseless for such a small timeout like one second.
    
    By increasing it to 24 hours, the kernel might cache lookup and getattr
    results, hence avoiding expensive online checks.

# Changes in 0.3.0

First public release on crates.io. No other changes.

# Changes in 0.2.0

-   Do not include . and .. for now
    
    These two entries are not strictly necessary and at the moment it
    interferes with the offset calculation.

-   Use offset parameter in readdir
    
    If it is not used, directories with many files are not displayed
    correctly.

# Changes in 0.1.0

Initial release, first working prototype of SplitFS.
