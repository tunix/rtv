#!/usr/bin/env python3
"""Update the <releases> section of the appstream metainfo from CHANGELOG.

release-please maintains CHANGELOG.md but never touches the metainfo, so the
packaged appstream data drifts behind the actual releases. This syncs
data/io.github.tunix.rtv.metainfo.xml from the changelog: one <release> entry
per version heading, newest first, capped at the 10 most recent.

Lives under .github/ because it exists only for the metainfo.yml workflow
(the top-level scripts/ directory was retired; repo rule: no one-off
scripts committed at the top level).

Usage: python3 .github/update_metainfo.py [changelog] [metainfo]
Exits 0 when the file is unchanged, 1 when it was rewritten.
"""
import re
import sys
from pathlib import Path
from xml.sax.saxutils import escape


def parse_changelog(text: str) -> list[dict]:
    """Extract (version, date, bullets) per '## [vX.Y.Z] (date)' heading."""
    releases = []
    # Headings look like: ## [1.3.1](link) (2026-10-13)  or  ## v0.3.0 (date)
    heading = re.compile(
        r"^## \[?v?([0-9]+\.[0-9]+\.[0-9]+[^\]]*)\]?\(?.*?\)?\s*\((\d{4}-\d{2}-\d{2})\)\s*$"
    )
    bullet = re.compile(r"^\s*[-*]\s+(.*)$")
    current = None
    for line in text.splitlines():
        m = heading.match(line)
        if m:
            if current:
                releases.append(current)
            current = {"version": m.group(1), "date": m.group(2), "bullets": []}
            continue
        if current:
            b = bullet.match(line)
            if b:
                current["bullets"].append(b.group(1).strip())
    if current:
        releases.append(current)
    return releases


def build_release_xml(rel: dict) -> str:
    version, date, bullets = rel["version"], rel["date"], rel["bullets"]
    if not bullets:
        return f'    <release version="{version}" date="{date}" />'
    items = "\n".join(f"          <li>{escape(b)}</li>" for b in bullets)
    return (
        f'    <release version="{version}" date="{date}">\n'
        f"      <description>\n        <ul>\n{items}\n        </ul>\n"
        f"      </description>\n    </release>"
    )


def main() -> int:
    changelog = Path(sys.argv[1] if len(sys.argv) > 1 else "CHANGELOG.md")
    metainfo = Path(sys.argv[2] if len(sys.argv) > 2
                    else "data/io.github.tunix.rtv.metainfo.xml")
    releases = parse_changelog(changelog.read_text())
    if not releases:
        print("no releases parsed from changelog — nothing to do")
        return 0
    # Appstream databases don't need the full history.
    releases = releases[:10]

    xml = metainfo.read_text()
    m = re.search(r"( *)<releases>.*?</releases>", xml, re.DOTALL)
    if not m:
        print("no <releases> block found in metainfo", file=sys.stderr)
        return 2
    new_block = ("<releases>\n"
                 + "\n".join(build_release_xml(r) for r in releases)
                 + "\n  </releases>")
    updated = xml[:m.start()] + "  " + new_block + xml[m.end():]
    if updated == xml:
        print("metainfo already up to date")
        return 0
    metainfo.write_text(updated)
    print(f"metainfo updated: {len(releases)} releases, latest "
          f"{releases[0]['version']} ({releases[0]['date']})")
    return 1


if __name__ == "__main__":
    sys.exit(main())
