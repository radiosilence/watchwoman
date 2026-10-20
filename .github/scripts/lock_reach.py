"""Whether a Cargo.lock change reaches what only the scarce runners build.

The Linux x86_64 jobs build the workspace for x86_64 glibc and musl. The
macOS and Linux arm64 runners are worth taking for a lock change only when it
touches a package those targets compile and x86_64 does not, such as the
FSEvents bindings. A change to a package every target compiles is checked on
x86_64, and main, which runs every platform for any Rust change, catches what
breaks on one platform alone. This prints `macos=true|false` and
`arm=true|false` for the change between two commits.

Packages are compared by name and version, since a lock can hold two
versions of one crate on different platforms. The platform closures come from `cargo metadata --filter-platform`, taken at
both commits so a package added or removed counts too. Any failure exits
non-zero, and the caller then treats the change as reaching everything.

    python3 lock_reach.py BASE HEAD
"""

import json
import subprocess
import sys
import tempfile
import tomllib

CHEAP = ["x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl"]
SCARCE = {
    "macos": ["aarch64-apple-darwin"],
    "arm": ["aarch64-unknown-linux-gnu", "aarch64-unknown-linux-musl"],
}


def show(rev, path):
    out = subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True, text=True)
    return out.stdout if out.returncode == 0 else ""


def changed_packages(old, new):
    def packages(text):
        return {(p["name"], p["version"]): p for p in tomllib.loads(text).get("package", [])} if text else {}

    before, after = packages(old), packages(new)
    changed = before.keys() ^ after.keys()
    changed |= {k for k in before.keys() & after.keys() if before[k] != after[k]}
    return changed


def closure(tree, target):
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=tree,
        capture_output=True,
        text=True,
        check=True,
    )
    meta = json.loads(out.stdout)
    keys = {p["id"]: (p["name"], p["version"]) for p in meta["packages"]}
    return {keys[n["id"]] for n in meta["resolve"]["nodes"]}


def only_on(tree):
    cheap = set().union(*(closure(tree, t) for t in CHEAP))
    return {
        domain: set().union(*(closure(tree, t) for t in targets)) - cheap
        for domain, targets in SCARCE.items()
    }


def main(base, head):
    changed = changed_packages(show(base, "Cargo.lock"), show(head, "Cargo.lock"))
    reach = dict.fromkeys(SCARCE, False)
    if changed:
        with tempfile.TemporaryDirectory() as tmp:
            trees = []
            for rev in (base, head):
                tree = f"{tmp}/{len(trees)}"
                subprocess.run(["git", "worktree", "add", "-q", "--detach", tree, rev], check=True)
                trees.append(tree)
            try:
                for tree in trees:
                    for domain, names in only_on(tree).items():
                        reach[domain] |= bool(changed & names)
            finally:
                for tree in trees:
                    subprocess.run(["git", "worktree", "remove", "--force", tree], check=False)
    for domain, value in reach.items():
        print(f"{domain}={'true' if value else 'false'}")


if __name__ == "__main__":
    main(*sys.argv[1:3])
