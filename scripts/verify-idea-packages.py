"""Verify the release archives before publishing either shell to a marketplace."""

import hashlib
import io
import json
from pathlib import Path
import xml.etree.ElementTree as ET
from zipfile import ZipFile

ROOT = Path(__file__).resolve().parents[1]
VERSION = json.loads((ROOT / "package.json").read_text())["version"]
DIST = ROOT / "idea/build/distributions"
TARGETS = [f"{os}-{arch}" for os in ("mac", "linux", "windows") for arch in ("arm64", "x86_64")]
MODULES = ("shared", "frontend", "backend", "backend.terminal", "browser.legacy", "browser.modular")


def descriptor(archive, entry, xml):
    with ZipFile(io.BytesIO(archive.read(entry))) as jar:
        return ET.fromstring(jar.read(xml))


def check_plugin(archive, version):
    root = descriptor(archive, f"acpira/lib/acpira-{VERSION}.jar", "META-INF/plugin.xml")
    assert root.findtext("id") == "com.github.hotic.acpira"
    assert root.findtext("version") == version
    assert {m.attrib["name"] for m in root.findall("content/module")} == {f"acpira.{m}" for m in MODULES}
    for module in MODULES:
        descriptor(archive, f"acpira/lib/modules/acpira.{module}.jar", f"acpira.{module}.xml")
    assert archive.read("acpira/sidecar/host-server.cjs")
    # JCEF's public content modules exist only from 262 onward; the legacy adapter remains separately loadable on 261.
    modern = descriptor(archive, "acpira/lib/modules/acpira.browser.modular.jar", "acpira.browser.modular.xml")
    deps = {d.attrib.get("name") for d in modern.findall("dependencies/module")}
    assert {"intellij.platform.ui.jcef", "intellij.libraries.jcef"} <= deps
    assert not root.findall("depends"), "Root OS constraints would prevent cross-platform client/backend installation"


def runtime(archive, prefix, target):
    exe = "node.exe" if target.startswith("windows-") else "node"
    entry = f"{prefix}/{exe}"
    assert archive.read(f"{prefix}/LICENSE")
    if exe == "node":
        assert archive.getinfo(entry).external_attr >> 16 & 0o111 == 0o111, f"Missing executable permissions: {entry}"
    return hashlib.sha256(archive.read(entry)).digest()


with ZipFile(DIST / f"acpira-{VERSION}.zip") as base:
    check_plugin(base, VERSION)
    assert not any(n.startswith("acpira/node/") for n in base.namelist())

with ZipFile(DIST / f"acpira-{VERSION}-universal.zip") as universal:
    check_plugin(universal, VERSION)
    backend = descriptor(universal, "acpira/lib/modules/acpira.backend.jar", "acpira.backend.xml")
    assert not any("com.intellij.modules.os." in d.attrib.get("id", "") for d in backend.findall("dependencies/plugin"))
    for target in TARGETS:
        with ZipFile(DIST / f"acpira-{VERSION}-{target}.zip") as variant:
            check_plugin(variant, f"{VERSION}-{target}")
            assert runtime(universal, f"acpira/node/{target}", target) == runtime(variant, "acpira/node", target)
            os, arch = target.split("-", 1)
            backend = descriptor(variant, "acpira/lib/modules/acpira.backend.jar", "acpira.backend.xml")
            deps = {d.attrib.get("id") for d in backend.findall("dependencies/plugin")}
            assert {f"com.intellij.modules.os.{os}", f"com.intellij.modules.arch.{arch}"} <= deps
            for module in MODULES:
                if module != "backend":
                    entry = f"acpira/lib/modules/acpira.{module}.jar"
                    assert variant.read(entry) == universal.read(entry), f"Stale module: {target}/{module}"
            print(f"Verified {target}: runtime, permissions, version and module consistency")

for line in (DIST / "SHA256SUMS").read_text().splitlines():
    expected, name = line.split("  ", 1)
    assert hashlib.sha256((DIST / name).read_bytes()).hexdigest() == expected, name
print(f"Verified Acpira {VERSION}: universal Marketplace package, six manual packages, base package and checksums")
