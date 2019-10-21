# Changes in 0.6.1-RC

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
