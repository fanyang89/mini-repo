# Mini Repo

One Rust process serves the upstream dufs file browser at `/`, a React SPA at
`/mini-repo/` for repository management and batch RPM upload, and one standard
DNF repository per sub-repository under `/repos/`. Each repository has a stable
ID (the DNF base URL), a display name and an architecture. The publisher uses `rpm`
and `rpmrepo_metadata` directly; no `createrepo_c` process or directory scan
is needed.

The management UI (`web/`) is a Vite + React + shadcn/ui SPA with a
master-detail layout: repository list and filters on the left, and per-repo
tabs (Packages, Jobs, Settings, DNF) on the right. It follows the system color
scheme with a manual light/dark toggle. `cargo build` embeds `web/dist` into
the binary: the crate's build script re-runs `pnpm build` whenever the SPA
sources change, so shipping stays a single executable.

## Run

Build on Linux with a C compiler, pkg-config and SQLite development headers.
CI uses Rust 1.100.0, Node.js 22.23.1 and pnpm 12.11.2.
From the repository root, build and initialize a new or empty base directory:

```sh
cargo build --release --locked
target/release/mini-repo create ./data
target/release/mini-repo serve ./data
```

Open http://127.0.0.1:5000/mini-repo/ and create a repository such as
`el9-x86_64`. Open **Upload**, select the repository and drop RPM files.
The original file browser is at http://127.0.0.1:5000/.

Edit `data/mini-repo.yaml` to change the listener and access rules; restart with
the same `serve` command. The default binds to loopback with uploads and
deletes enabled. For network deployment, use dufs HTTPS or a TLS reverse
proxy and configure accounts in that file:

```yaml
allow-upload: true
allow-delete: true
bind: 127.0.0.1
auth:
  - 'editor:REPLACE_WITH_PASSWORD@/mini-repo:rw,/incoming:rw,/repos:ro'
  - '@/repos:ro'
```

This allows authenticated Web UI management and anonymous DNF downloads.
For a per-repository uploader, grant `/mini-repo:rw`, `/incoming/ID:rw`
and `/repos/ID:ro`; creating repositories additionally requires write
permission on `/incoming`. Keep the config and state outside the served root.
Use trusted uploaders for ordinary files served on the same web origin.
`serve` accepts the original network and authentication flags, including
`--path-prefix`. The legacy dufs invocation and `MINI_REPO_STATE` still work.

Create, edit or delete sub-repositories at `/mini-repo/`. Repository IDs
are stable DNF URLs; the display name is free text (it becomes the `name=`
field of the `.repo` stanza), and architecture can be changed while the
sub-repository is empty. Delete an individual RPM
from its search result: the background publisher removes it from that repo’s
index and unlinks its stored file. Deleting a sub-repository removes all of
its packages, partials and jobs. Destructive actions require delete permission.

## Upload and publication

- A successful `PUT /incoming/ID/file.rpm` queues a complete RPM. The dedicated
  uploader groups successful transfers by batch, then submits them together.
  A lost browser session leaves completed items **ready** for manual submission.
- Resume with `HEAD` for the partial length and `PATCH` with `X-Update-Range:
  append`. The dedicated UI supplies the expected offset and complete length.
  A plain dufs/WebDAV `PATCH` without a total remains partial. Confirm it in
  **Recent uploads** using the original byte length. Invalid digests fail import.
- The publisher coalesces queued changes for two seconds and processes one
  repository at a time. Repeated identical NEVRAs are idempotent; conflicting
  bytes or a mismatched architecture produce a failed job. `noarch` is accepted
  only into the selected repository. Retry failed jobs in the management UI.
  **Rebuild index** explicitly rereads that repository's stored RPMs; ordinary
  uploads do not take this path.
- New RPMs are hashed and checked against their SHA-256 payload digest or legacy
  header+payload MD5. These checks detect corruption, not publisher identity.
  RPM signing policy remains with the DNF client.
- Old packages stream from the previous XML indexes. The publisher writes a
  new gzip `primary`, `filelists` and `other` set. SQLite commits the current
  manifest pointer, searchable catalog and completed jobs together. Failure
  leaves the previous repository visible; older metadata URLs remain readable
  for seven days after replacement. Ordinary RPM downloads bypass the worker.
- DNF uses `/repos/ID/` as `baseurl`. The catalog provides a copyable `.repo`
  stanza. Search across readable repositories or filter by architecture.

## Catalog, OLAP and SBOM

The SQLite catalog at `<base-path>/state/catalog.sqlite` is the structured
source of truth. Besides the serving tables it carries, per package: name,
epoch/version/release/arch, license, vendor, packager, url, summary, source
RPM, build time and sizes, plus `requires` / `provides` rows for dependency
graphs, per-package `files` rows, and an append-only `events` table
(`add` / `remove` with timestamps).
Rows are written inside the publish transaction; **Rebuild index** refreshes
the enriched columns of pre-existing packages. Package search runs on an FTS5
trigram index in the same database — substring semantics, case-insensitive,
updated atomically with each publish (queries under three characters fall
back to a plain scan).

### OLAP with DuckDB

```sh
duckdb
```
```sql
INSTALL sqlite; LOAD sqlite;
ATTACH 'data/state/catalog.sqlite' AS catalog (TYPE SQLITE, READ_ONLY);

-- version spread of one package across repositories
SELECT repo, version, arch FROM catalog.packages WHERE name='nginx' ORDER BY repo;

-- duplicate NEVRAs shipped by more than one repository
SELECT nevra, count(DISTINCT repo) repos FROM catalog.packages
GROUP BY nevra HAVING repos > 1;

-- which repository ships a given file
SELECT repo, nevra FROM catalog.files WHERE path='/usr/bin/curl';

-- additions and removals per repository over time
SELECT repo, date(ts, 'unixepoch') day, action, count(*) FROM catalog.events
GROUP BY 1, 2, 3 ORDER BY day DESC;
```

The server keeps the database in WAL mode, so DuckDB can read it while the
service runs. For long-running analysis copy the file first.

### SBOM export

`GET /mini-repo/api/sbom?repo=ID` returns an SPDX 2.3 JSON document generated
from the catalog (one SPDX package per RPM, SHA-256 checksums, RPM license
tags, vendors and `pkg:rpm` purls). Read permission for the repository is
required — the same as for package listings.

## Storage and scope

`<base-path>/files/` is the dufs root. `<base-path>/state/` contains the SQLite catalog, partials, durable job spool,
RPMs and revision directories. Use a local filesystem supporting hard links;
run one publisher per state directory. Stop the service before copying the
entire state directory for a backup. `mini-repo/`, `repos/` and `incoming/`
are reserved HTTP namespaces; existing unmanaged repositories are not imported.
Failed jobs retain their uploads for retry. This version publishes standard
package indexes, not modularity, advisories, or signed `repomd.xml`.

## Project layout

| Path | Start here for |
| --- | --- |
| `Cargo.toml` | Workspace members and Mini Repo product version. The default member builds the server binary. |
| `vendor/dufs/src/` | Product entry point: `main.rs` starts the server; `mini.rs` connects its HTTP routes to the core. |
| `mini-repo/src/` | `mini-repo-core`: storage and SQLite catalog in `lib.rs`, RPM publication in `publish.rs`. |
| `web/` | React management UI. See [frontend development](web/README.md) for the dev server and proxy. |
| `tests/`, `web/tests/`, `vendor/dufs/tests/` | Product smoke test, browser flow and Rust integration tests, respectively. |

The server is a modified copy of [dufs](https://github.com/sigoden/dufs)
0.47.0 at `77291a230565ca090a1d7751d07db82ac418906d`. Its file-browser
assets are unmodified.

## Verification

```sh
cargo build --locked
pnpm --dir web lint
cargo test --workspace --locked
pnpm --dir web exec playwright install --with-deps --only-shell chromium
python3 tests/smoke.py --ui
```

The smoke test requires `rpmbuild` and `dnf`. It builds five RPMs in a temporary
directory, starts a private loopback server, and checks batched publication,
resume, restart, repository isolation, corruption, old manifest URLs,
download checksums, access rules and DNF package discovery. `--ui` also drives
the SPA through create, edit, upload and deletion using the same server and
one of those RPMs. Omit `--ui` to run the smoke test without a browser.
Both use `target/debug/mini-repo` by default; pass `--binary PATH` to test
another build.

## License

Mini Repo's original code is licensed under the [MIT License](LICENSE).
The upstream dufs code retains its [MIT](vendor/dufs/LICENSE-MIT) or
[Apache-2.0](vendor/dufs/LICENSE-APACHE) licensing.
