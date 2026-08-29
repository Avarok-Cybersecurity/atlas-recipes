#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Every internal version requirement must equal the package it points at.

With ``--fix``, rewrites them instead of complaining. The release workflow uses
that: release-please bumps ``[package] version`` and never touches the
requirements pointing at it, so on the release branch they are stale by
construction. Repairing them there is the same job as repairing the lock, and
it makes a split release harmless — each requirement gets the version of the
crate it actually names, whether or not the crates moved together.

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


def linked_components(cfg: dict) -> list[str]:
    """The components the linked-versions plugin claims to group."""
    for plugin in cfg.get("plugins", []):
        if isinstance(plugin, dict) and plugin.get("type") == "linked-versions":
            return plugin.get("components", [])
    return []


def check_release_config() -> int:
    """Every linked component must name a crate release-please can see.

    A component that matches nothing is not an error to release-please — it is
    silently ignored, and the group it was meant to join simply does not form.
    That is how five crates ended up proposed at two different versions with
    `linked-versions` sitting right there in the config looking correct:
    `atlas-recipes-data` matched nothing, because path "." was registered under
    the name `atlasctl`, which is also the name of a real crate one directory
    down.
    """
    cfg_path = ROOT / "release-please-config.json"
    cfg = json.loads(cfg_path.read_text())

    known = set()
    root_name = cfg.get("packages", {}).get(".", {}).get("package-name")
    if root_name:
        known.add(root_name)
    for manifest in sorted(ROOT.glob("crates/*/Cargo.toml")):
        with manifest.open("rb") as fh:
            known.add(tomllib.load(fh)["package"]["name"])

    # The name release-please uses for "." should be the crate that is there,
    # or the component list cannot refer to both it and its namesake.
    with (ROOT / "Cargo.toml").open("rb") as fh:
        actual_root = tomllib.load(fh)["package"]["name"]
    bad = 0
    if root_name != actual_root:
        print(
            f"::error::release-please-config.json calls path \".\" {root_name!r}, "
            f"but the crate there is {actual_root!r}"
        )
        bad = 1

    for component in linked_components(cfg):
        if component in known:
            print(f"ok   linked component {component}")
        else:
            print(f"::error::linked-versions names {component!r}, which is no crate here")
            bad = 1

    # Naming a real crate is not enough: release-please only bumps paths listed
    # in `packages`. A component that is a real crate but NOT a package is
    # grouped with things that move and then never moves itself -- silently,
    # because an unlisted path is not an error to release-please either.
    #
    # That is not hypothetical. It froze `crates/atlasctl` at 0.1.7 while the
    # root went to 0.4.1, so `atlasctl --version` reported the same string for
    # builds an entire wire-protocol revision apart -- and the installer, which
    # compared those strings, told an operator "already installed here" forever
    # while the control page kept telling them to upgrade. The version string
    # cannot be an identity for a build if releases never move it.
    declared = {}
    for path, spec in cfg.get("packages", {}).items():
        name = spec.get("package-name")
        if not name:
            manifest = ROOT / path / "Cargo.toml"
            if manifest.exists():
                with manifest.open("rb") as fh:
                    name = tomllib.load(fh)["package"]["name"]
        if name:
            declared[name] = path
    for component in linked_components(cfg):
        if component in declared:
            print(f"ok   {component} is a released package ({declared[component]})")
        else:
            print(
                f"::error::linked-versions groups {component!r}, but no `packages` "
                f"entry covers it -- release-please will never bump it, so it will "
                f"sit at whatever version it has today while the group moves on"
            )
            bad = 1
    return bad


def as_tuple(version: str) -> tuple[int, ...]:
    """A dotted version as integers, for ordering.

    Only ever used to answer "is the manifest behind the package". These are
    plain `x.y.z` values written by release-please; anything unparseable sorts
    lowest, so a value this cannot read is treated as stale rather than
    silently accepted as current.
    """
    return tuple(
        int(chunk) if chunk.isdigit() else 0
        for chunk in version.split("-", 1)[0].split(".")
    )


def rewrite_manifest_entry(manifest_file: pathlib.Path, path: str, new: str) -> None:
    """Record `path` as `new` in the release manifest, in place.

    Line-oriented for the same reason the requirements are: a JSON round-trip
    would reorder and reformat a file release-please also writes, and a diff
    nobody can read is a diff nobody reviews.
    """
    lines = manifest_file.read_text().splitlines(keepends=True)
    needle = f'"{path}":'
    for i, line in enumerate(lines):
        if line.lstrip().startswith(needle):
            head, _, tail = line.partition(needle)
            suffix = "," if tail.rstrip().endswith(",") else ""
            lines[i] = f'{head}{needle} "{new}"{suffix}\n'
            manifest_file.write_text("".join(lines))
            return


def rewrite_requirement(manifest: pathlib.Path, name: str, old: str, new: str) -> bool:
    """Point one requirement at a new version, in place.

    Line-oriented and anchored on the dependency name, because rewriting the
    file through a TOML serialiser would reformat and strip comments — and the
    comments here are load-bearing.
    """
    lines = manifest.read_text().splitlines(keepends=True)
    needle = f'version = "{old}"'
    for i, line in enumerate(lines):
        stripped = line.lstrip()
        if stripped.startswith(f"{name} ") and needle in line:
            lines[i] = line.replace(needle, f'version = "{new}"', 1)
            manifest.write_text("".join(lines))
            return True
    return False


def main() -> int:
    fix = "--fix" in sys.argv
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
                if fix and rewrite_requirement(manifest, name, required, actual):
                    print(f"fix  {rel} [{table}] {name} {required} -> {actual}")
                else:
                    print(
                        f"::error::{rel} [{table}] requires {name} = \"{required}\" "
                        f"but {target.relative_to(ROOT)} is {actual}"
                    )
                    bad = 1
            else:
                print(f"ok   {rel} [{table}] {name} {required}")

    # `--fix` used to skip this half outright, on the reasoning that the release
    # branch records released-from versions until the release is cut. That
    # premise was wrong, and the release PR proves it: `chore: release main`
    # updated four of the five entries in the same commit that bumped their
    # Cargo manifests, and left the fifth behind. So the manifest *is* written
    # when the PR is prepared — just not always completely.
    #
    # Direction is what decides whether repairing is safe.
    #
    # An entry *ahead* of its Cargo.toml is release-please mid-generation.
    # Rewriting it would undo a bump it is in the middle of making, so it stays
    # a check even under `--fix`.
    #
    # An entry *behind* its Cargo.toml is not transient, and it is quietly
    # destructive. The manifest is what the next release computes from, so a
    # stale entry makes the next run propose a version that is already on
    # crates.io — and crates.io versions are immutable. Nothing fails now; the
    # release fails permanently, later, from a state no rerun can clear. That
    # is what `atlasctl-protocol` was about to do at 0.1.3 with its package
    # already at 0.2.0.
    manifest_file = ROOT / ".release-please-manifest.json"
    for path, recorded in json.loads(manifest_file.read_text()).items():
        target = (ROOT / path / "Cargo.toml") if path != "." else (ROOT / "Cargo.toml")
        actual = package_version(target)
        if actual == recorded:
            print(f"ok   release manifest {path} {recorded}")
            continue
        if fix and actual is not None and as_tuple(recorded) < as_tuple(actual):
            rewrite_manifest_entry(manifest_file, path, actual)
            print(f"fix  release manifest {path} {recorded} -> {actual}")
            continue
        print(
            f"::error::.release-please-manifest.json records {path} as {recorded} "
            f"but {target.relative_to(ROOT)} is {actual}"
        )
        bad = 1

    bad |= check_release_config()

    if fix and not bad:
        return 0

    if bad:
        print(
            "::error::Set each requirement equal to the package it names. "
            "A minor release cannot resolve a stale one, and it fails after the "
            "release PR is already open."
        )
    return bad


if __name__ == "__main__":
    sys.exit(main())
