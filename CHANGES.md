# Changes since latest release

-   Do not include . and .. for now
    
    These two entries are not strictly necessary and at the moment it
    interferes with the offset calculation.

-   Use offset parameter in readdir
    
    If it is not used, directories with many files are not displayed
    correctly.

# Changes in 0.1.0

Initial release, first working prototype of SplitFS.
