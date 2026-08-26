#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Every internal version requirement must equal the package it points at.

Workspace members depend on each other by path *and* version, because
publishing to crates.io needs the version. Nothing in cargo keeps that
requirement in step with the package it names.

The drift is invisible while both sides are 0.1.x: `^0.1.2` resolves against
0.1.3 quite happily. It only bites when a release cuts a minor bump, because
`^0.1.2` means `>=0.1.2, <0.2.0` — so 0.2.0 cannot satisfy it and the release
job dies with `failed to select a version for the requirement`, *after* the
release PR is open and looking healthy. That blocked every release attempt on
2026-08-26, and it hid in five separate places.

This parses TOML rather than grepping. The first version of this check used a
regex that assumed `path` came before `version` and only read the workspace
root, and it sailed straight past

    atlasctl-protocol = { version = "0.1.2", path = "../atlasctl-protocol" }

in a member crate. Key order is not semantic in TOML, and a check that depends
on it is a check that lies.

The same reasoning covers .release-please-manifest.json: release-please
computes the next version from that file, not from the manifests on disk, so a
stale entry there gives one crate a different bump from the rest — which is
what split the release into 0.2.0 and 0.1.4 and defeated linked-versions.
"""

from __future__ import annotations

import json
import pathlib
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")


def package_version(manifest: pathlib.Path) -> str | None:
    """The `[package] version` a manifest declares, if it declares one."""
    with manifest.open("rb") as fh:
        data = tomllib.load(fh)
    return data.get("package", {}).get("version")


def path_dependencies(manifest: pathlib.Path):
    """Every dependency in this manifest that names a path and a version."""
    with manifest.open("rb") as fh:
        data = tomllib.load(fh)

    tables = [(t, data.get(t, {})) for t in DEP_TABLES]
    tables.append(("workspace.dependencies", data.get("workspace", {}).get("dependencies", {})))

    for table, deps in tables:
        for name, spec in deps.items():
            if not isinstance(spec, dict):
                continue
            path, version = spec.get("path"), spec.get("version")
            # A path with no version is fine in-tree and simply cannot be
            # published; cargo says so far more clearly than this would.
            if path and version:
                yield table, name, path, version


def main() -> int:
    manifests = [ROOT / "Cargo.toml", *sorted(ROOT.glob("crates/*/Cargo.toml"))]
    bad = 0

    for manifest in manifests:
        rel = manifest.relative_to(ROOT)
        for table, name, path, required in path_dependencies(manifest):
            target = (manifest.parent / path / "Cargo.toml").resolve()
            actual = package_version(target)
            if actual is None:
                print(f"::error::{rel} [{table}] {name} points at {path}, which declares no version")
                bad = 1
            elif required != actual:
                print(
                    f"::error::{rel} [{table}] requires {name} = \"{required}\" "
                    f"but {target.relative_to(ROOT)} is {actual}"
                )
                bad = 1
            else:
                print(f"ok   {rel} [{table}] {name} {required}")

    manifest_file = ROOT / ".release-please-manifest.json"
    for path, recorded in json.loads(manifest_file.read_text()).items():
        target = (ROOT / path / "Cargo.toml") if path != "." else (ROOT / "Cargo.toml")
        actual = package_version(target)
        if actual != recorded:
            print(
                f"::error::.release-please-manifest.json records {path} as {recorded} "
                f"but {target.relative_to(ROOT)} is {actual}"
            )
            bad = 1
        else:
            print(f"ok   release manifest {path} {recorded}")

    if bad:
        print(
            "::error::Set each requirement equal to the package it names. "
            "A minor release cannot resolve a stale one, and it fails after the "
            "release PR is already open."
        )
    return bad


if __name__ == "__main__":
    sys.exit(main())
