#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Print the workspace's publishable crates in dependency order.

crates.io refuses a crate whose path dependencies are not already published, so
`cargo publish` has to walk the workspace bottom-up. The release workflow used
to carry that order as a hand-written list, which listed three of the five
crates and put atlasctl-core before atlasctl-protocol that it depends on — a
publish that could never have succeeded. Deriving the order from the manifests
means adding a crate cannot silently reintroduce that.

Reads `cargo metadata --no-deps` on stdin, writes one crate name per line.
Exits non-zero on a dependency cycle rather than emitting a partial order.
"""

import json
import sys


def main() -> int:
    meta = json.load(sys.stdin)
    # `publish = false` marks a crate as deliberately unpublishable; absent or
    # true means it ships. Anything else is a registry allow-list, still ships.
    packages = {
        p["name"]: p
        for p in meta["packages"]
        if p.get("publish") is None or p.get("publish")
    }
    edges = {
        name: {d["name"] for d in pkg["dependencies"] if d["name"] in packages}
        for name, pkg in packages.items()
    }

    order: list[str] = []
    done: set[str] = set()

    def visit(name: str, stack: tuple[str, ...] = ()) -> None:
        if name in done:
            return
        if name in stack:
            raise SystemExit("dependency cycle: " + " -> ".join(stack + (name,)))
        for dep in sorted(edges[name]):
            visit(dep, stack + (name,))
        done.add(name)
        order.append(name)

    # Sorted entry points so the output is byte-stable across runs.
    for name in sorted(packages):
        visit(name)

    print("\n".join(order))
    return 0


if __name__ == "__main__":
    sys.exit(main())
