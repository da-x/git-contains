## `git-contains` - to which remote branches my commit went?

This is a tool designed to analyze a list of remote Git branches and identify
all commits made by a specific user across those branches. It examines the
commit history to locate user contributions in each branch, accounting for
branches that contain merge histories from others. The tool filters out
duplicate commits and generates a clear table that shows which recent commits
are present in each branch, providing a concise overview of the user's work
throughout the repository.

Example output:

```
2023.01.07 11:42:56 | ┊┊┊┊┊xxxxxx┊ 5938f0b4b371 Some change
2023.01.07 12:01:28 | ┊┊┊┊┊xxxxxx┊ 4143552dc4b1 Here's the first commit message line
2023.01.07 20:43:54 | ┊x┊┊┊xxxxxxx e9ec39df023e Another fix
2023.01.21 14:16:22 | ┊┊┊┊┊x┊┊┊┊┊┊ efeb46e75308 Small fix
2023.01.22 18:13:14 | ┊┊┊┊┊┊┊x┊┊┊┊ b0fb9c9ef760 Unrelated fix
2023.01.26 07:05:01 | x┊xxx┊x┊┊x┊x b2b47e0c18cb Important change
2023.01.26 07:05:15 | x┊xxx┊x┊┊x┊x 0d18b6e4d567 Important fix to change
2023.01.26 18:04:19 | ┊┊xxx┊x┊┊x┊┊ 2ada4cc22187 Some other fix to that
                      ││││││││││││
                      │││││││││││Pipeline 1321 [420a2001136]
                      ││││││││││main
                      │││││││││stable
                      ││││││││dev
                      │││││││stable/feature-b-24.2
                      ││││││stable/feature-a-24.2
                      │││││stable/feature-c-24.2
                      ││││dev/24.1.0-hf2
                      │││dev/24.1.0-hf
                      ││dev/24.1.0-sp
                      │dev/24.1.0-sp2
                      dev/24.0.0-sp3
```


### Command line

```
git-contains 0.1.0

USAGE:
    git-contains [FLAGS] [OPTIONS] [--] [git-dir]

FLAGS:
    -h, --help        Prints help information
    -r, --reverse     Reverse the display order
    -v, --variants    Show all the variants of commits having the same commit subject line
    -V, --version     Prints version information

OPTIONS:
        --author <author>       Author to sort by
        --branch <branch>...    Branches to show
    -d, --days <days>           Alternative git directory to use [default: 30]
        --search <search>       Highlight certain commits containing given text

ARGS:
    <git-dir>    Alternative git directory to use
```


### Installation

Install after [Rust toolchain](https://www.rust-lang.org/tools/install) with `cargo install --path .`


### Ref script

Optionally, you configure a script that will resolve "branches" such as
`<thing>:<param>` to something more meaningful, for example from 'pipe:1234567'
to the hash corresponding to Gitlab pipeline 1234567.

```
[contains]
        refscript = git-contains-ref-script
```

Then your `git-contains-ref-script` script can be as such:

```sh
#!/bin/bash

set -eu
set -o pipefail

main() {
    if [[ "$1" =~ ^pipe:([0-9]+)$ ]] ; then
        local pipeline="${BASH_REMATCH[1]}"
	local githash=$(pipe2hash ${pipeline} | cut -c1-12)
        echo "Pipeline ${pipeline} [${githash}]"
        echo $(pipe2hash ${pipeline} | cut -c1-12)
    fi
}

main "$@"
```
