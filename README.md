# gitman

Manage multiple git repositories that live in the same parent directory using
**super branches** — named sets of (repository, branch) pairs defined in a
`gitman.toml` file. Repositories not listed in a super branch are left untouched.

## Setup

Place a `gitman.toml` in the parent directory of your repositories:

```
workspace/
├── gitman.toml
├── auth-service/
├── web-frontend/
└── billing/            # not listed anywhere -> never touched
```

```toml
[superbranches.feature-login]
auth-service = "feature/login"
web-frontend = "feature/login-ui"

[superbranches.hotfix-42]
auth-service = "hotfix/42"
```

## Usage

Run from the parent directory (or point at it with `-C <dir>`):

```sh
gitman list                     # print the active gitman.toml
gitman edit                     # open gitman.toml in $VISUAL/$EDITOR (fallback: vi)
gitman checkout feature-login   # git checkout the mapped branch in each listed repo
gitman pull feature-login       # checkout, git pull --ff-only, restore previous branch
gitman push feature-login       # checkout, git push, restore previous branch
gitman status                   # current branch of every repo (marks dirty trees)
gitman status feature-login     # compare checked out branches against a super branch
gitman build my-feature         # snapshot the checked out branches as a new super
                                # branch and append it to gitman.toml
gitman update my-feature        # re-snapshot the checked out branches into an
                                # existing super branch
gitman --help                   # full help; also: gitman help <command>
```

`build` scans every direct subdirectory that is a git repository, records its
currently checked out branch, and appends the result as a new
`[superbranches.<name>]` block — existing content and comments in gitman.toml
are preserved. Repos with a detached HEAD are skipped with a warning. Edit the
generated block afterwards to drop repos that don't belong to the feature.

`pull` and `push` remember the branch that was checked out before, run the
operation on the configured branch, and then switch back — your working state
is restored even if the pull or push fails.

Errors in one repository (dirty working tree, missing branch, diverged pull)
are reported and the remaining repositories are still processed; the exit code
is non-zero if anything failed.

## Build

```sh
cargo build --release           # binary at target/release/gitman
cargo install --path .          # or install it onto your PATH
```
