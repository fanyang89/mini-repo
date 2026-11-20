"""Run against the built binary, real RPMs and DNF (requires rpmbuild and dnf)."""

import argparse
import base64
import hashlib
import json
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
from urllib import request, error
from xml.etree import ElementTree as ET


def build_rpms(work):
    spec = work / "example.spec"
    names = ["demo"] + [f"demo-{n}" for n in range(1, 5)]
    lines = [
        "Name: demo",
        "Version: 1",
        "Release: 1",
        "Summary: Smoke test",
        "License: MIT",
        "BuildArch: noarch",
        "%description",
        "Smoke test",
    ]
    for name in names[1:]:
        lines += [
            f"%package -n {name}",
            "Summary: Smoke test",
            f"%description -n {name}",
            "Smoke test",
        ]
    lines += ["%install", "mkdir -p %{buildroot}/usr/share/demo"]
    for name in names:
        lines.append(f"echo test > %{{buildroot}}/usr/share/demo/{name}")
    for name in names:
        lines += [f"%files -n {name}", f"/usr/share/demo/{name}"]
    spec.write_text("\n".join(lines) + "\n")
    subprocess.run(
        [
            "rpmbuild",
            "--define",
            f"_topdir {work}",
            "--define",
            "_smp_build_ncpus 2",
            "-bb",
            spec,
        ],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    return sorted((work / "RPMS/noarch").glob("*.rpm"))


def smoke(binary, work, ui=False):
    rpms = build_rpms(work)
    assert len(rpms) == 5
    base = work / "base"
    assert subprocess.run([binary, "serve", base], capture_output=True).returncode != 0
    subprocess.run([binary, "create", base], check=True)
    assert subprocess.run([binary, "create", base], capture_output=True).returncode != 0
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    url = f"http://127.0.0.1:{port}"
    auth = "Basic " + base64.b64encode(b"editor:example").decode()

    def http(path, method="GET", data=None, headers=None, status=200, admin=True):
        h = {"Authorization": auth} if admin else {}
        req = request.Request(
            url + path, method=method, data=data, headers=h | (headers or {})
        )
        try:
            resp = request.urlopen(req, timeout=5)
        except error.HTTPError as e:
            resp = e
        with resp:
            body = resp.read()
            assert resp.status == status, (method, path, resp.status, body)
            return body

    def api(name, params, post=False, **kwargs):
        from urllib.parse import urlencode

        body = http(
            "/mini-repo/api/" + name + "?" + urlencode(params),
            "POST" if post else "GET",
            headers={"X-Mini-Repo": "1"},
            **kwargs,
        )
        return json.loads(body) if kwargs.get("status", 200) == 200 else body

    def start():
        return subprocess.Popen(
            [
                binary,
                "serve",
                base,
                "-b",
                "127.0.0.1",
                "-p",
                str(port),
                "-a",
                "editor:example@/:rw",
                "-a",
                "limited:example@/mini-repo:rw,/incoming/a:rw,/repos/a:ro",
                "-a",
                "@/mini-repo,/repos/a",
            ],
        )

    def wait(f):
        end = time.monotonic() + 20
        while time.monotonic() < end:
            try:
                if f():
                    return
            except (OSError, ConnectionError):
                pass
            time.sleep(0.1)
        raise AssertionError("timed out")

    def stop(p):
        p.terminate()
        p.wait(5)

    def packages(repo="a"):
        return api("packages", {"repo": repo})

    def revision(repo):
        return next(
            x["repo"]["revision"] for x in api("repos", {}) if x["repo"]["id"] == repo
        )

    p = start()
    try:
        wait(lambda: http("/__dufs__/health"))
        if ui:
            script = Path(__file__).resolve().parents[1] / "web/tests/ui.mjs"
            subprocess.run(
                ["node", script, url, rpms[0], auth], check=True, timeout=120
            )
        assert b"__INDEX_DATA__" not in http("/") and b"DUFS" in http("/").upper()
        # SPA shell plus its hashed bundle, with a history-routing fallback.
        page = http("/mini-repo/upload")
        assert b'id="root"' in page and b"./assets/" in page
        script = page.decode().split('src="./')[1].split('"')[0]
        bundle = http(f"/mini-repo/{script}")
        assert len(bundle) > 100000
        http("/mini-repo/some/route", status=200)
        http("/regular.txt", "PUT", b"hello", status=201)
        assert http("/regular.txt") == b"hello"
        for repo, arch in [("a", "x86_64"), ("b", "aarch64")]:
            api(
                "create",
                dict(repo=repo, name=f"Repo {repo}", arch=arch),
                True,
            )
        wait(lambda: revision("b"))
        limited = "Basic " + base64.b64encode(b"limited:example").decode()
        assert [
            x["repo"]["id"]
            for x in json.loads(
                http("/mini-repo/api/repos", headers={"Authorization": limited})
            )
        ] == ["a"]
        http("/incoming/b/no.rpm", "PUT", b"no", {"Authorization": limited}, 403)
        http(
            "/mini-repo/api/rebuild?repo=b",
            "POST",
            headers={"Authorization": limited, "X-Mini-Repo": "1"},
            status=403,
        )
        unchanged = revision("b")
        api(
            "create",
            dict(repo="no", name="x", arch="noarch"),
            True,
            admin=False,
            status=401,
        )
        for f in rpms[:4]:
            http(
                "/incoming/a/" + f.name,
                "PUT",
                f.read_bytes(),
                {"X-Mini-Batch": "test"},
                202,
            )
        last = rpms[-1].read_bytes()
        cut = len(last) // 2
        target = "/incoming/a/" + rpms[-1].name
        http(target, "PUT", last[:cut], {"X-Mini-Total": str(len(last))}, 409)
        assert not packages()
        stop(p)
        p = start()
        wait(lambda: http("/__dufs__/health"))
        assert len(api("jobs", {"repo": "a"})["jobs"]) == 4
        api("submit", {"repo": "a", "batch": "test"}, True)
        wait(lambda: len(packages()) == 4)
        first = revision("a")
        old = http("/repos/a/repodata/repomd.xml")
        http(target, "PATCH", last[cut:], {"X-Update-Range": "append"}, 202)
        assert len(packages()) == 4
        http(
            target,
            "POST",
            headers={"X-Mini-Total": str(len(last)), "X-Mini-Repo": "1"},
            status=202,
        )
        wait(lambda: len(packages()) == 5)
        second = revision("a")
        assert first != second and unchanged == revision("b")
        for loc in ET.fromstring(old).findall(".//{*}location"):
            assert http("/repos/a/" + loc.attrib["href"])
        http(target, "PUT", last, status=202)
        http("/incoming/a/broken.rpm", "PUT", last[:-30], status=202)
        wait(
            lambda: any(
                j["status"] == "failed" for j in api("jobs", {"repo": "a"})["jobs"]
            )
        )
        assert second == revision("a")
        # Enriched catalog: license/vendor columns, dependency rows and add/remove events.
        with sqlite3.connect(
            f"file:{base}/state/catalog.sqlite?mode=ro", uri=True
        ) as db:
            rich = db.execute(
                "SELECT name,license,sourcerpm FROM packages WHERE repo='a' AND license!='MIT'"
            ).fetchall()
            assert not rich, rich
            assert (
                db.execute("SELECT count(*) FROM provides WHERE repo='a'").fetchone()[0]
                >= 5
            )
            assert (
                db.execute(
                    "SELECT count(*) FROM events WHERE repo='a' AND action='add'"
                ).fetchone()[0]
                == 5
            )
            # One shipped file per built RPM.
            assert (
                db.execute(
                    "SELECT count(*) FROM files WHERE repo='a' AND path LIKE '/usr/share/demo/%'"
                ).fetchone()[0]
                == 5
            )
        # FTS search: substring, case-insensitive, punctuation-safe, short fallback.
        assert (
            len(api("packages", {"repo": "a", "q": "demo-1"})) == 2
        )  # demo and demo-1
        assert len(api("packages", {"repo": "a", "q": "DEMO"})) == 5
        assert (
            api("packages", {"repo": "a", "q": "1:2"}) == []
        )  # no epoch in these RPMs
        assert len(api("packages", {"repo": "a", "q": "de"})) == 5
        assert api("packages", {"repo": "a", "q": "%"}) == []
        sbom = json.loads(http("/mini-repo/api/sbom?repo=a", admin=False))
        assert sbom["spdxVersion"] == "SPDX-2.3"
        assert len(sbom["packages"]) == 5
        assert all(p["checksums"][0]["algorithm"] == "SHA256" for p in sbom["packages"])
        assert len(sbom["relationships"]) == 5
        assert any(
            "pkg:rpm/demo@" in p["externalRefs"][0]["referenceLocator"]
            for p in sbom["packages"]
        )
        api("rebuild", {"repo": "a"}, True)
        wait(lambda: revision("a") != second)
        assert unchanged == revision("b") and len(packages()) == 5
        # Rebuild refreshed the catalog columns of pre-existing packages.
        with sqlite3.connect(
            f"file:{base}/state/catalog.sqlite?mode=ro", uri=True
        ) as db:
            assert (
                db.execute(
                    "SELECT count(*) FROM packages WHERE repo='a' AND name=''"
                ).fetchone()[0]
                == 0
            )
        for pkg in packages():
            actual = http("/repos/a/" + pkg["href"], admin=False)
            assert hashlib.sha256(actual).hexdigest() == pkg["href"].split("/")[1]
        dnf = [
            "dnf",
            "-q",
            "--repofrompath",
            f"smoke,{url}/repos/a/",
            "--releasever=9",
            "--setopt=cachedir=" + str(work / "dnf-cache"),
            "--setopt=persistdir=" + str(work / "dnf-state"),
            "--setopt=logdir=" + str(work / "dnf-log"),
            "repoquery",
            "--repo=smoke",
            "--qf",
            "%{name}\\n",
        ]
        log = subprocess.check_output(dnf, text=True)
        assert set(log.split()) == {"demo", "demo-1", "demo-2", "demo-3", "demo-4"}, log
        # A completed browser batch makes exactly one generation, even across a restart.
        generations = base / "state/repos/b/md"
        before = set(generations.iterdir())
        for f in rpms:
            http(
                "/incoming/b/" + f.name,
                "PUT",
                f.read_bytes(),
                {"X-Mini-Batch": "five"},
                202,
            )
        api("submit", {"repo": "b", "batch": "five"}, True)
        stop(p)
        p = start()
        wait(lambda: len(packages("b")) == 5)
        assert len(set(generations.iterdir()) - before) == 1
        with sqlite3.connect(base / "state/catalog.sqlite") as db:
            assert (
                db.execute(
                    "SELECT count(*) FROM jobs WHERE repo='b' AND status='published'"
                ).fetchone()[0]
                == 5
            )
        current_b = revision("b")
        fields = dict(repo="a", name="edited", arch="x86_64")
        api("update", fields, True)
        assert (
            next(x["repo"] for x in api("repos", {}) if x["repo"]["id"] == "a")["name"]
            == "edited"
        )
        api("update", fields | {"arch": "aarch64"}, True, status=409)
        victim = packages()[0]
        api("delete-rpm", {"repo": "a", "href": victim["href"]}, True)
        wait(lambda: len(packages()) == 4)
        http("/repos/a/" + victim["href"], status=404)
        assert not (base / "state/repos/a" / victim["href"]).exists()
        with sqlite3.connect(
            f"file:{base}/state/catalog.sqlite?mode=ro", uri=True
        ) as db:
            assert (
                db.execute(
                    "SELECT count(*) FROM events WHERE repo='a' AND action='remove'"
                ).fetchone()[0]
                == 1
            )
            assert (
                db.execute(
                    "SELECT count(*) FROM requires WHERE repo='a' AND nevra=?",
                    (victim["nevra"],),
                ).fetchone()[0]
                == 0
            )
            assert (
                db.execute(
                    "SELECT count(*) FROM files WHERE repo='a' AND nevra=?",
                    (victim["nevra"],),
                ).fetchone()[0]
                == 0
            )
        assert (
            len(
                subprocess.check_output(
                    [*dnf[:1], "--refresh", *dnf[1:]], text=True
                ).split()
            )
            == 4
        )
        http(
            "/mini-repo/api/delete-repo?repo=a",
            "POST",
            headers={"Authorization": limited, "X-Mini-Repo": "1"},
            status=403,
        )
        api("delete-repo", {"repo": "a"}, True)
        http("/repos/a/repodata/repomd.xml", status=404)
        assert revision("b") == current_b and len(packages("b")) == 5
        stop(p)
        p = start()
        wait(lambda: len(api("repos", {})) == 1)
        api("create", fields, True)
        wait(lambda: revision("a"))
        assert packages() == []
    finally:
        stop(p)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/debug/mini-repo"))
    parser.add_argument(
        "--ui", action="store_true", help="also exercise the SPA in Chromium"
    )
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        smoke(args.binary.resolve(), Path(tmp), args.ui)
    print("OK: uploads, restart, incremental publication, isolation, permissions, DNF")
