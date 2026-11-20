//! Incremental publication of independent RPM repositories.
mod publish;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, Notify, RwLock};
use uuid::Uuid;

use include_dir::{Dir, include_dir as embed_dir};

pub const SPA: Dir<'static> = embed_dir!("$CARGO_MANIFEST_DIR/../web/dist");

pub fn spa_file(path: &str) -> Option<&'static [u8]> {
    SPA.get_file(path).map(|f| f.contents())
}
const REBUILD: &str = "[Rebuild index]";
const REMOVE: &str = "[Delete RPM] ";

pub fn component(s: &str) -> bool {
    (1..=180).contains(&s.len())
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
}

#[derive(Serialize)]
pub struct Repo {
    pub id: String,
    pub name: String,
    pub arch: String,
    pub revision: Option<String>,
    pub packages: u64,
}
#[derive(Serialize)]
pub struct Job {
    pub id: String,
    pub filename: String,
    pub status: String,
    pub detail: String,
}
#[derive(Serialize)]
pub struct Entry {
    pub nevra: String,
    pub filename: String,
    pub href: String,
    pub name: String,
    pub license: String,
    pub sourcerpm: String,
    pub build_time: i64,
}

pub struct Store {
    pub lifecycle: RwLock<()>,
    state: PathBuf,
    root: PathBuf,
    _lock: fs::File,
    // Bound concurrent writes to the same partial, including its final handoff.
    pub upload_locks: [Mutex<()>; 128],
    wake: Notify,
}

struct CatalogRow {
    nevra: String,
    name: String,
    version: String,
    release: String,
    epoch: String,
    arch: String,
    sha: String,
    license: String,
    vendor: String,
    url: String,
    sourcerpm: String,
    href: String,
}

impl Store {
    pub fn open(state: &Path, root: &Path) -> Result<Arc<Self>> {
        fs::create_dir_all(state)?;
        let state = state.canonicalize()?;
        let root = root.canonicalize()?;
        ensure!(
            !state.starts_with(&root) && !root.starts_with(&state),
            "state and served root must be disjoint"
        );
        let lock = fs::File::create(state.join("instance.lock"))?;
        lock.try_lock()
            .context("another publisher is using this state directory")?;
        for name in ["mini-repo", "repos"] {
            let path = root.join(name);
            if !path.exists() {
                fs::create_dir(&path)?;
            }
            ensure!(
                path.canonicalize()? == path && fs::read_dir(&path)?.next().is_none(),
                "{name}/ is reserved: use an empty directory"
            );
        }
        let s = Arc::new(Self {
            lifecycle: RwLock::new(()),
            state,
            root,
            _lock: lock,
            upload_locks: std::array::from_fn(|_| Mutex::new(())),
            wake: Notify::new(),
        });
        let db = s.db()?;
        db.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS repos(id TEXT PRIMARY KEY,name TEXT NOT NULL DEFAULT '',arch TEXT NOT NULL,revision TEXT);
            CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,repo TEXT NOT NULL,filename TEXT NOT NULL,status TEXT NOT NULL,detail TEXT NOT NULL DEFAULT '',batch TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS pending ON jobs(status,repo);
            CREATE TABLE IF NOT EXISTS packages(repo TEXT NOT NULL,nevra TEXT NOT NULL,filename TEXT NOT NULL,href TEXT NOT NULL,sha TEXT NOT NULL,PRIMARY KEY(repo,nevra));
            CREATE UNIQUE INDEX IF NOT EXISTS package_href ON packages(repo,href);
            CREATE TABLE IF NOT EXISTS requires(repo TEXT NOT NULL,nevra TEXT NOT NULL,name TEXT NOT NULL,flags TEXT,epoch TEXT,version TEXT,release TEXT);
            CREATE INDEX IF NOT EXISTS requires_lookup ON requires(name);
            CREATE TABLE IF NOT EXISTS provides(repo TEXT NOT NULL,nevra TEXT NOT NULL,name TEXT NOT NULL,flags TEXT,epoch TEXT,version TEXT,release TEXT);
            CREATE INDEX IF NOT EXISTS provides_lookup ON provides(name);
            CREATE TABLE IF NOT EXISTS files(repo TEXT NOT NULL,nevra TEXT NOT NULL,path TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS files_path ON files(path);
            CREATE INDEX IF NOT EXISTS files_pkg ON files(repo,nevra);
            CREATE TABLE IF NOT EXISTS events(ts INTEGER NOT NULL,repo TEXT NOT NULL,action TEXT NOT NULL,nevra TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS events_repo ON events(repo,ts);
            CREATE TABLE IF NOT EXISTS deleting(repo TEXT PRIMARY KEY);")?;
        // Catalog enrichment columns, added incrementally so existing data survives.
        let existing: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('packages')")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let enriched = [
            ("name", "TEXT NOT NULL DEFAULT ''"),
            ("epoch", "TEXT NOT NULL DEFAULT ''"),
            ("version", "TEXT NOT NULL DEFAULT ''"),
            ("release", "TEXT NOT NULL DEFAULT ''"),
            ("arch", "TEXT NOT NULL DEFAULT ''"),
            ("license", "TEXT NOT NULL DEFAULT ''"),
            ("vendor", "TEXT NOT NULL DEFAULT ''"),
            ("packager", "TEXT NOT NULL DEFAULT ''"),
            ("url", "TEXT NOT NULL DEFAULT ''"),
            ("summary", "TEXT NOT NULL DEFAULT ''"),
            ("sourcerpm", "TEXT NOT NULL DEFAULT ''"),
            ("build_time", "INTEGER NOT NULL DEFAULT 0"),
            ("size_package", "INTEGER NOT NULL DEFAULT 0"),
            ("size_installed", "INTEGER NOT NULL DEFAULT 0"),
        ];
        for (column, kind) in enriched {
            if !existing.iter().any(|c| c == column) {
                db.execute_batch(&format!("ALTER TABLE packages ADD COLUMN {column} {kind}"))?;
            }
        }
        // Created here, after the name column is guaranteed to exist.
        db.execute_batch("CREATE INDEX IF NOT EXISTS package_name ON packages(repo,name)")?;
        // Trigram FTS keeps substring semantics (LIKE '%q%') while using an
        // index; queries shorter than three characters fall back to a scan.
        db.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS packages_fts USING fts5(nevra,filename,summary,content='packages',content_rowid='rowid',tokenize='trigram');
             CREATE TRIGGER IF NOT EXISTS packages_fts_ai AFTER INSERT ON packages BEGIN
               INSERT INTO packages_fts(rowid,nevra,filename,summary) VALUES(new.rowid,new.nevra,new.filename,new.summary);
             END;
             CREATE TRIGGER IF NOT EXISTS packages_fts_ad AFTER DELETE ON packages BEGIN
               INSERT INTO packages_fts(packages_fts,rowid,nevra,filename,summary) VALUES('delete',old.rowid,old.nevra,old.filename,old.summary);
             END;
             CREATE TRIGGER IF NOT EXISTS packages_fts_au AFTER UPDATE OF nevra,filename,summary ON packages BEGIN
               INSERT INTO packages_fts(packages_fts,rowid,nevra,filename,summary) VALUES('delete',old.rowid,old.nevra,old.filename,old.summary);
               INSERT INTO packages_fts(rowid,nevra,filename,summary) VALUES(new.rowid,new.nevra,new.filename,new.summary);
             END;",
        )?;
        let (indexed, total): (i64, i64) = db.query_row(
            "SELECT (SELECT count(*) FROM packages_fts),(SELECT count(*) FROM packages)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if indexed != total {
            db.execute_batch("INSERT INTO packages_fts(packages_fts) VALUES('rebuild')")?;
        }
        // Migrate pre-name schemas: distro/release were display-only labels.
        let columns: Vec<String> = db
            .prepare("SELECT name FROM pragma_table_info('repos')")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if columns.iter().any(|c| c == "distro") {
            db.execute_batch(
                "ALTER TABLE repos DROP COLUMN distro; ALTER TABLE repos DROP COLUMN release;",
            )?;
        }
        if !columns.iter().any(|c| c == "name") {
            db.execute_batch("ALTER TABLE repos ADD COLUMN name TEXT NOT NULL DEFAULT ''")?;
        }
        let retired: Vec<String> = db
            .prepare("SELECT repo FROM deleting")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in retired {
            s.cleanup_repo(&id)?;
        }
        let handoffs: Vec<(String, String)> = db
            .prepare("SELECT id,repo FROM jobs WHERE status='receiving'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (id, repo) in handoffs {
            if s.dir(&repo).join("queue").join(&id).is_file() {
                s.finish_handoff(&db, &id)?;
            } else {
                db.execute("DELETE FROM jobs WHERE id=?", [id])?;
            }
        }
        let worker = s.clone();
        tokio::spawn(async move {
            loop {
                // Check at startup too: a committed queue survives a process restart.
                tokio::time::sleep(Duration::from_secs(2)).await;
                let w = worker.clone();
                match tokio::task::spawn_blocking(move || w.run_pending()).await {
                    Ok(Ok(())) => worker.wake.notified().await,
                    err => log::error!("RPM publisher: {err:?}"),
                }
            }
        });
        Ok(s)
    }
    fn db(&self) -> Result<Connection> {
        let db = Connection::open(self.state.join("catalog.sqlite"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        Ok(db)
    }
    fn dir(&self, repo: &str) -> PathBuf {
        self.state.join("repos").join(repo)
    }
    pub fn partial(&self, repo: &str, file: &str) -> PathBuf {
        self.dir(repo).join("partial").join(file)
    }
    pub fn exists(&self, repo: &str) -> Result<bool> {
        Ok(self.db()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM repos WHERE id=?)",
            [repo],
            |r| r.get(0),
        )?)
    }
    pub fn create(&self, id: &str, name: &str, arch: &str) -> Result<()> {
        ensure!(
            component(id) && component(arch) && name.chars().count() <= 200,
            "invalid repository fields"
        );
        ensure!(!self.exists(id)?, "repository already exists");
        ensure!(
            !self.db()?.query_row(
                "SELECT EXISTS(SELECT 1 FROM deleting WHERE repo=?)",
                [id],
                |r| r.get::<_, bool>(0)
            )?,
            "repository deletion still pending"
        );
        let incoming = self.root.join("incoming");
        fs::create_dir_all(&incoming)?;
        ensure!(
            incoming.canonicalize()? == incoming,
            "incoming must not be a symlink"
        );
        let entry = incoming.join(id);
        if !entry.exists() {
            fs::create_dir(&entry)?;
        }
        ensure!(
            entry.canonicalize()? == entry && fs::read_dir(&entry)?.next().is_none(),
            "incoming directory must be empty"
        );
        fs::create_dir_all(self.dir(id).join("partial"))?;
        fs::create_dir_all(self.dir(id).join("queue"))?;
        fs::create_dir_all(self.dir(id).join("Packages"))?;
        self.db()?.execute(
            "INSERT INTO repos(id,name,arch) VALUES(?,?,?)",
            params![id, name, arch],
        )?;
        self.wake.notify_one();
        Ok(())
    }
    pub fn update(&self, id: &str, name: &str, arch: &str) -> Result<()> {
        ensure!(
            component(id) && component(arch) && name.chars().count() <= 200,
            "invalid repository fields"
        );
        // Keep the published URL stable, and don't relabel an incompatible package set.
        let db = self.db()?;
        ensure!(!db.query_row("SELECT EXISTS(SELECT 1 FROM repos WHERE id=?1 AND arch!=?2 AND EXISTS(SELECT 1 FROM packages WHERE repo=?1))",params![id,arch],|r|r.get::<_,bool>(0))?,"empty the repository before changing its architecture");
        ensure!(
            db.execute(
                "UPDATE repos SET name=?,arch=? WHERE id=?",
                params![name, arch, id]
            )? == 1,
            "unknown repository"
        );
        Ok(())
    }
    pub fn delete_repo(&self, id: &str) -> Result<()> {
        ensure!(component(id), "invalid repository ID");
        let mut db = self.db()?;
        let tx = db.transaction()?;
        ensure!(
            tx.execute("DELETE FROM repos WHERE id=?", [id])? == 1,
            "unknown repository"
        );
        tx.execute("DELETE FROM packages WHERE repo=?", [id])?;
        tx.execute("DELETE FROM jobs WHERE repo=?", [id])?;
        tx.execute("INSERT INTO deleting VALUES(?)", [id])?;
        tx.commit()?;
        self.cleanup_repo(id)
    }
    fn cleanup_repo(&self, id: &str) -> Result<()> {
        let dir = self.dir(id);
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        let inbox = self.root.join("incoming").join(id);
        if inbox.is_dir() {
            // The public marker can contain unrelated files placed by a local operator.
            match fs::remove_dir(inbox) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => (),
                Err(e) => return Err(e.into()),
            }
        }
        self.db()?
            .execute("DELETE FROM deleting WHERE repo=?", [id])?;
        Ok(())
    }
    pub fn remove_package(&self, repo: &str, href: &str) -> Result<String> {
        let db = self.db()?;
        let sha: String = db
            .query_row(
                "SELECT sha FROM packages WHERE repo=? AND href=?",
                params![repo, href],
                |r| r.get(0),
            )
            .context("package is not published in this repository")?;
        let id = Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO jobs(id,repo,filename,status,batch) VALUES(?,?,?,'queued','')",
            params![id, repo, format!("{REMOVE}{sha}")],
        )?;
        self.wake.notify_one();
        Ok(id)
    }
    pub fn repos(&self) -> Result<Vec<Repo>> {
        Ok(self
            .db()?
            .prepare(
                "SELECT r.id,r.name,r.arch,r.revision,count(p.nevra)
                 FROM repos r LEFT JOIN packages p ON p.repo=r.id GROUP BY r.id ORDER BY r.id",
            )?
            .query_map([], |r| {
                Ok(Repo {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    arch: r.get(2)?,
                    revision: r.get(3)?,
                    packages: r.get::<_, i64>(4)? as u64,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn entries(&self, repo: &str, q: &str, offset: u32) -> Result<Vec<Entry>> {
        // Trigram FTS needs at least three characters; shorter or empty
        // queries use the plain LIKE scan below.
        if q.chars().count() >= 3 {
            // Quote the whole input so user text like "1:2.3" is matched
            // literally instead of being parsed as FTS query syntax.
            let match_query = format!("\"{}\"", q.replace('"', "\"\""));
            let sql = "SELECT p.nevra,p.filename,p.name,p.license,p.sourcerpm,p.build_time,p.href
                 FROM packages_fts f JOIN packages p ON p.rowid=f.rowid
                 WHERE f.packages_fts MATCH ?1 AND p.repo=?2 ORDER BY p.nevra LIMIT 100 OFFSET ?3";
            return Ok(self
                .db()?
                .prepare(sql)?
                .query_map(params![match_query, repo, offset], Self::entry_row)?
                .collect::<rusqlite::Result<_>>()?);
        }
        // LIKE wildcards in the user input must not widen the match.
        let escaped: String = q
            .chars()
            .flat_map(|c| match c {
                '\\' | '%' | '_' => vec!['\\', c],
                _ => vec![c],
            })
            .collect();
        let like = format!("%{escaped}%");
        Ok(self
            .db()?
            .prepare(
                "SELECT nevra,filename,name,license,sourcerpm,build_time,href FROM packages
                 WHERE repo=?1 AND (nevra LIKE ?2 ESCAPE '\\' OR filename LIKE ?2 ESCAPE '\\')
                 ORDER BY nevra LIMIT 100 OFFSET ?3",
            )?
            .query_map(params![repo, like, offset], Self::entry_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn entry_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
        Ok(Entry {
            nevra: r.get(0)?,
            filename: r.get(1)?,
            name: r.get(2)?,
            license: r.get(3)?,
            sourcerpm: r.get(4)?,
            build_time: r.get(5)?,
            href: r.get(6)?,
        })
    }

    /// SPDX 2.3 SBOM for one repository, generated from the catalog.
    pub fn sbom(&self, repo: &str, origin: &str, baseurl: &str) -> Result<String> {
        let rows: Vec<CatalogRow> = self
            .db()?
            .prepare(
                "SELECT nevra,name,version,release,epoch,arch,sha,license,vendor,url,sourcerpm,href FROM packages WHERE repo=? ORDER BY nevra",
            )?
            .query_map([repo], |r| {
                Ok(CatalogRow {
                    nevra: r.get(0)?,
                    name: r.get(1)?,
                    version: r.get(2)?,
                    release: r.get(3)?,
                    epoch: r.get(4)?,
                    arch: r.get(5)?,
                    sha: r.get(6)?,
                    license: r.get(7)?,
                    vendor: r.get(8)?,
                    url: r.get(9)?,
                    sourcerpm: r.get(10)?,
                    href: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut packages = Vec::new();
        let mut relationships = Vec::new();
        for CatalogRow {
            nevra,
            name,
            version,
            release,
            epoch,
            arch,
            sha,
            license,
            vendor,
            url,
            sourcerpm,
            href,
        } in rows
        {
            let id = format!("SPDXRef-Package-{}", &sha[..12.min(sha.len())]);
            let mut evr = version.clone();
            if !release.is_empty() {
                evr = format!("{evr}-{release}");
            }
            let mut purl = format!("pkg:rpm/{name}@{evr}?arch={arch}");
            if !epoch.is_empty() && epoch != "0" {
                purl.push_str(&format!("&epoch={epoch}"));
            }
            packages.push(serde_json::json!({
                "SPDXID": id,
                "name": name,
                "versionInfo": evr,
                "filesAnalyzed": false,
                "licenseConcluded": "NOASSERTION",
                "licenseDeclared": if license.is_empty() { "NOASSERTION".to_string() } else { license },
                "downloadLocation": format!("{baseurl}{href}"),
                "checksums": [{ "algorithm": "SHA256", "checksumValue": sha }],
                "supplier": if vendor.is_empty() { serde_json::Value::Null } else { serde_json::json!(format!("Organization: {vendor}")) },
                "homepage": if url.is_empty() { serde_json::Value::Null } else { serde_json::json!(url) },
                "sourceInfo": if sourcerpm.is_empty() { serde_json::Value::Null } else { serde_json::json!(format!("Built from {sourcerpm}")) },
                "externalRefs": [{
                    "referenceCategory": "PACKAGE-MANAGER",
                    "referenceType": "purl",
                    "referenceLocator": purl,
                }],
                "comment": nevra,
            }));
            relationships.push(serde_json::json!({
                "spdxElementId": "SPDXRef-DOCUMENT",
                "relationshipType": "DESCRIBES",
                "relatedSpdxElement": id,
            }));
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let document = serde_json::json!({
            "spdxVersion": "SPDX-2.3",
            "dataLicense": "CC0-1.0",
            "SPDXID": "SPDXRef-DOCUMENT",
            "name": format!("{repo}-sbom"),
            "documentNamespace": format!("{origin}/mini-repo/api/sbom?repo={repo}&t={}", now),
            "creationInfo": {
                "created": now,
                "creators": ["Tool: mini-repo"],
            },
            "packages": packages,
            "relationships": relationships,
        });
        Ok(serde_json::to_string_pretty(&document)?)
    }
    pub fn jobs(&self, repo: &str) -> Result<Vec<Job>> {
        Ok(self.db()?.prepare("SELECT id,filename,status,detail FROM jobs WHERE repo=? ORDER BY rowid DESC LIMIT 100")?.query_map([repo],|r|Ok(Job{id:r.get(0)?,filename:r.get(1)?,status:r.get(2)?,detail:r.get(3)?}))?.collect::<rusqlite::Result<_>>()?)
    }
    pub fn partials(&self, repo: &str) -> Result<Vec<String>> {
        fs::read_dir(self.dir(repo).join("partial"))?
            .map(|e| Ok(e?.file_name().to_string_lossy().into_owned()))
            .collect()
    }
    pub fn enqueue(&self, repo: &str, file: &str, batch: &str) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let partial = self.partial(repo, file);
        let saved = self.dir(repo).join("queue").join(&id);
        let db = self.db()?;
        db.execute(
            "INSERT INTO jobs(id,repo,filename,status,batch) VALUES(?,?,?,'receiving',?)",
            params![id, repo, file, batch],
        )?;
        if let Err(e) = fs::rename(partial, saved) {
            db.execute("DELETE FROM jobs WHERE id=?", [&id])?;
            return Err(e.into());
        }
        self.finish_handoff(&db, &id)?;
        if batch.is_empty() {
            self.wake.notify_one();
        }
        Ok(id)
    }
    fn finish_handoff(&self, db: &Connection, id: &str) -> Result<()> {
        db.execute(
            "UPDATE jobs SET status=CASE batch WHEN '' THEN 'queued' ELSE 'ready' END WHERE id=?",
            [id],
        )?;
        Ok(())
    }
    pub fn submit(&self, repo: &str, batch: &str) -> Result<()> {
        self.db()?.execute(
            "UPDATE jobs SET status='queued' WHERE repo=? AND batch=? AND status='ready'",
            params![repo, batch],
        )?;
        self.wake.notify_one();
        Ok(())
    }
    pub fn retry(&self, repo: &str, id: &str) -> Result<()> {
        ensure!(self.db()?.execute("UPDATE jobs SET status='queued',detail='' WHERE repo=? AND id=? AND status IN ('ready','failed')",params![repo,id])? == 1,"job is not ready or failed");
        self.wake.notify_one();
        Ok(())
    }
    pub fn rebuild(&self, repo: &str) -> Result<()> {
        self.db()?.execute(
            "INSERT INTO jobs(id,repo,filename,status,batch) VALUES(?,?,?,'queued','')",
            params![Uuid::new_v4().to_string(), repo, REBUILD],
        )?;
        self.wake.notify_one();
        Ok(())
    }
    pub fn download(&self, repo: &str, path: &[&str]) -> Result<Option<PathBuf>> {
        if !component(repo) {
            return Ok(None);
        }
        let base = self.dir(repo);
        Ok(match path {
            ["repodata", "repomd.xml"] => self
                .db()?
                .query_row("SELECT revision FROM repos WHERE id=?", [repo], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .optional()?
                .flatten()
                .map(|rev| base.join("md").join(rev).join("public.xml")),
            ["repodata", key] => key.split_once('_').and_then(|(rev, name)| {
                (Uuid::parse_str(rev).is_ok()
                    && ["primary.xml.gz", "filelists.xml.gz", "other.xml.gz"].contains(&name))
                .then(|| base.join("md").join(rev).join("repodata").join(name))
            }),
            ["Packages", sha, file]
                if sha.len() == 64
                    && sha.bytes().all(|b| b.is_ascii_hexdigit())
                    && component(file) =>
            {
                let href = format!("Packages/{sha}/{file}");
                self.db()?
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM packages WHERE repo=? AND href=?)",
                        params![repo, href],
                        |r| r.get::<_, bool>(0),
                    )?
                    .then(|| base.join(href))
            }
            _ => None,
        })
    }
    fn run_pending(&self) -> Result<()> {
        let _guard = self.lifecycle.blocking_read();
        let db = self.db()?;
        let repos:Vec<(String,String)> = db.prepare("SELECT id,arch FROM repos WHERE revision IS NULL OR id IN (SELECT repo FROM jobs WHERE status='queued')")?.query_map([],|r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (repo, arch) in repos {
            let mut jobs: Vec<(String, String)> = db
                .prepare(
                    "SELECT id,filename FROM jobs WHERE repo=? AND status='queued' ORDER BY rowid",
                )?
                .query_map([&repo], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            // Keep deletes ordered relative to uploads of the same NEVRA.
            if let Some((_, first)) = jobs.first() {
                let removing = first.starts_with(REMOVE);
                if let Some(split) = jobs
                    .iter()
                    .position(|(_, f)| f.starts_with(REMOVE) != removing)
                {
                    jobs.truncate(split);
                    self.wake.notify_one();
                }
            }
            if let Err(e) = self.publish(&repo, &arch, &jobs) {
                log::error!("publish {repo}: {e:#}");
                for (id, _) in &jobs {
                    db.execute(
                        "UPDATE jobs SET status='failed', detail=? WHERE id=? AND status='queued'",
                        params![format!("{e:#}"), id],
                    )?;
                }
            }
        }
        Ok(())
    }
}
