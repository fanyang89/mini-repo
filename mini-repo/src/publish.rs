use super::*;
use rpm::{IndexSignatureTag, IndexTag, PackageMetadata};
use rpmrepo_metadata::{
    Checksum, CompressionType, Package, PackageSource, RepomdData, RepomdXml, RepositoryOptions,
    RepositoryReader, RepositoryWriter, Requirement,
};
use rusqlite::Transaction;

// Inside the publish transaction a cached prepared statement keeps dependency
// loading cheap even for packages with thousands of entries.
fn write_dependencies(
    tx: &Transaction,
    repo: &str,
    nevra: &str,
    table: &str,
    deps: &[Requirement],
) -> Result<()> {
    let mut statement = tx.prepare_cached(&format!(
        "INSERT INTO {table}(repo,nevra,name,flags,epoch,version,release) VALUES(?,?,?,?,?,?,?)"
    ))?;
    for dep in deps {
        let flags = dep.flags().map(|f| f.as_str().to_string());
        statement.execute(params![
            repo,
            nevra,
            dep.name(),
            flags,
            dep.epoch(),
            dep.version(),
            dep.release(),
        ])?;
    }
    Ok(())
}

// Same pattern as dependencies: one cached statement inside the publish
// transaction; a package with tens of thousands of files stays cheap.
fn write_files(tx: &Transaction, repo: &str, nevra: &str, pkg: &Package) -> Result<()> {
    let mut statement = tx.prepare_cached("INSERT INTO files(repo,nevra,path) VALUES(?,?,?)")?;
    for file in pkg.files().iter() {
        statement.execute(params![repo, nevra, file.path()])?;
    }
    Ok(())
}

use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{BufReader, Read, Write},
    time::SystemTime,
};

// Validate the compressed payload while hashing the file; never decompress or buffer the RPM.
fn read_package(path: &Path, filename: &str) -> Result<Package> {
    let header = PackageMetadata::open(path)?;
    let off = header.get_package_segment_offsets();
    let file = fs::File::open(path)?;
    let meta = file.metadata()?;
    ensure!(meta.len() > off.payload, "truncated RPM");
    let expected = header
        .header
        .get_entry_data_as_string_array(IndexTag::RPMTAG_PAYLOADSHA256)
        .ok()
        .and_then(|v| v.first().copied())
        .or_else(|| {
            header
                .header
                .get_entry_data_as_string(IndexTag::RPMTAG_PAYLOADSHA256)
                .ok()
        });
    let legacy = header
        .signature
        .get_entry_data_as_binary(IndexSignatureTag::RPMSIGTAG_MD5)
        .ok();
    let header_checks = header.check_digests()?;
    if header_checks.has_header_digest() {
        header_checks.result()?;
    }
    ensure!(
        expected.is_some() || legacy.is_some(),
        "RPM needs a SHA-256 payload digest or legacy header+payload MD5"
    );
    let (mut full, mut payload, mut md5) = (Sha256::new(), Sha256::new(), md5::Context::new());
    let mut reader = BufReader::new(file);
    let mut buf = [0u8; 65536];
    let mut pos = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        full.update(&buf[..n]);
        if expected.is_some() {
            payload.update(&buf[(off.payload.saturating_sub(pos).min(n as u64) as usize)..n]);
        }
        if expected.is_none() && legacy.is_some() {
            md5.consume(&buf[(off.header.saturating_sub(pos).min(n as u64) as usize)..n]);
        }
        pos += n as u64;
    }
    if let Some(exp) = expected {
        ensure!(
            exp.eq_ignore_ascii_case(&hex::encode(payload.finalize())),
            "payload checksum mismatch"
        );
    } else if let Some(exp) = legacy {
        ensure!(
            exp == md5.finalize().as_slice(),
            "header/payload checksum mismatch"
        );
    }
    let sha = hex::encode(full.finalize());
    let source = PackageSource {
        checksum: Some(Checksum::Sha256(sha.clone())),
        size_package: Some(pos),
        time_file: meta
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs(),
        location_href: format!("Packages/{sha}/{filename}"),
        location_base: None,
    };
    let p = Package::from_package_metadata(&header, source, Default::default())?;
    p.validate_xml_fields()?;
    Ok(p)
}

impl Store {
    pub(super) fn publish(&self, repo: &str, arch: &str, jobs: &[(String, String)]) -> Result<()> {
        let base = self.dir(repo);
        let mut db = self.db()?;
        let old: Option<String> =
            db.query_row("SELECT revision FROM repos WHERE id=?", [repo], |r| {
                r.get(0)
            })?;
        let mut added = HashMap::<String, Package>::new();
        let mut removed = HashMap::<String, String>::new();
        let mut backfill = Vec::<Package>::new();
        let mut done = Vec::new();
        let rebuild = jobs.iter().any(|(_, f)| f == REBUILD);
        for (id, file) in jobs {
            if let Some(sha) = file.strip_prefix(REMOVE) {
                let href = db
                    .query_row(
                        "SELECT href FROM packages WHERE repo=? AND sha=?",
                        params![repo, sha],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?;
                if let Some(href) = href {
                    removed.insert(sha.to_owned(), href);
                }
                done.push(id);
                continue;
            }
            if file == REBUILD {
                done.push(id);
                continue;
            }
            let spool = base.join("queue").join(id);
            let checked = (|| -> Result<Package> {
                let p = read_package(&spool, file)?;
                ensure!(
                    p.arch() == arch || p.arch() == "noarch",
                    "package architecture {} does not match {arch}",
                    p.arch()
                );
                Ok(p)
            })();
            let p = match checked {
                Ok(p) => p,
                Err(e) => {
                    db.execute(
                        "UPDATE jobs SET status='failed',detail=? WHERE id=?",
                        params![e.to_string(), id],
                    )?;
                    continue;
                }
            };
            let nevra = p.nevra();
            let existing = if let Some(previous) = added.get(&nevra) {
                Some(previous.pkgid().to_owned())
            } else {
                db.query_row(
                    "SELECT sha FROM packages WHERE repo=? AND nevra=?",
                    params![repo, nevra],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
            };
            if let Some(sha) = existing {
                if sha != p.pkgid() {
                    db.execute("UPDATE jobs SET status='failed',detail='same NEVRA with different content' WHERE id=?",[id])?;
                    continue;
                }
            } else {
                let dest = base.join(p.location_href());
                fs::create_dir_all(dest.parent().unwrap())?;
                if !dest.exists() {
                    fs::hard_link(&spool, dest)?;
                }
                added.insert(nevra, p);
            }
            done.push(id);
        }
        let mut revision = None;
        if !added.is_empty() || !removed.is_empty() || old.is_none() || rebuild {
            let rev = Uuid::new_v4().to_string();
            let stage = base.join("md").join(&rev);
            let prev = old
                .as_ref()
                .filter(|_| !rebuild)
                .map(|r| {
                    RepositoryReader::new_from_directory(&base.join("md").join(r))?.iter_packages()
                })
                .transpose()?;
            let existing = if rebuild {
                db.query_row("SELECT count(*) FROM packages WHERE repo=?", [repo], |r| {
                    r.get::<_, i64>(0)
                })?
                .try_into()?
            } else {
                prev.as_ref().map_or(0, |p| p.total_packages())
            };
            let count = existing
                .checked_sub(removed.len())
                .context("catalog and metadata disagree; rebuild index")?
                + added.len();
            let options = RepositoryOptions::default().compression_type(CompressionType::Gzip);
            let mut writer = RepositoryWriter::new_with_options(&stage, count, options)?;
            if rebuild {
                let mut statement = db.prepare(
                    "SELECT filename,href,sha FROM packages WHERE repo=? ORDER BY nevra",
                )?;
                let mut rows = statement.query([repo])?;
                while let Some(row) = rows.next()? {
                    let filename: String = row.get(0)?;
                    let href: String = row.get(1)?;
                    let p = read_package(&base.join(href), &filename)?;
                    ensure!(
                        p.pkgid() == row.get::<_, String>(2)?,
                        "stored package checksum changed"
                    );
                    writer.add_package(&p)?;
                    backfill.push(p);
                }
            }
            if let Some(prev) = prev {
                for p in prev {
                    let p = p?;
                    if !removed.contains_key(p.pkgid()) {
                        writer.add_package(&p)?;
                    }
                }
            }
            for p in added.values() {
                writer.add_package(p)?;
            }
            writer.finish()?;
            // Keep the library's relative manifest for the next incremental read.
            // Public URLs include the revision, so cached manifests stay valid.
            let generated = RepositoryReader::new_from_directory(&stage)?;
            let mut manifest = RepomdData::default();
            manifest.set_revision(&rev);
            for mut rec in generated.repomd().records().iter().cloned() {
                let filename = rec
                    .location_href
                    .file_name()
                    .context("missing metadata filename")?
                    .to_string_lossy();
                rec.location_href = Path::new("repodata").join(format!("{rev}_{filename}"));
                manifest.add_record(rec);
            }
            let (_, mut xml) = rpmrepo_metadata::utils::xml_writer_for_path(
                &stage.join("public.xml"),
                CompressionType::None,
            )?;
            RepomdXml::write_data(&manifest, &mut xml)?;
            xml.get_mut().flush()?;
            if let Some(previous) = &old {
                fs::File::open(base.join("md").join(previous))?.set_modified(SystemTime::now())?;
            }
            revision = Some(rev);
        }
        let tx = db.transaction()?;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs() as i64;
        for href in removed.values() {
            let nevra: Option<String> = tx
                .query_row(
                    "SELECT nevra FROM packages WHERE repo=? AND href=?",
                    params![repo, href],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(nevra) = nevra else { continue };
            tx.execute(
                "DELETE FROM packages WHERE repo=? AND nevra=?",
                params![repo, &nevra],
            )?;
            tx.execute(
                "DELETE FROM requires WHERE repo=? AND nevra=?",
                params![repo, &nevra],
            )?;
            tx.execute(
                "DELETE FROM provides WHERE repo=? AND nevra=?",
                params![repo, &nevra],
            )?;
            tx.execute(
                "DELETE FROM files WHERE repo=? AND nevra=?",
                params![repo, &nevra],
            )?;
            tx.execute(
                "INSERT INTO events(ts,repo,action,nevra) VALUES(?1,?,'remove',?)",
                params![now, repo, nevra],
            )?;
        }
        for (nevra, p) in &added {
            let href = p.location_href();
            let file = href.rsplit('/').next().unwrap();
            tx.execute(
                "INSERT INTO packages VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    repo,
                    nevra,
                    file,
                    href,
                    p.pkgid(),
                    p.name(),
                    p.epoch(),
                    p.version(),
                    p.release(),
                    p.arch(),
                    p.rpm_license(),
                    p.rpm_vendor(),
                    p.packager(),
                    p.url(),
                    p.summary(),
                    p.rpm_sourcerpm(),
                    p.time_build() as i64,
                    p.size_package() as i64,
                    p.size_installed() as i64,
                ],
            )?;
            write_dependencies(&tx, repo, nevra, "requires", p.requires())?;
            write_dependencies(&tx, repo, nevra, "provides", p.provides())?;
            write_files(&tx, repo, nevra, p)?;
            tx.execute(
                "INSERT INTO events(ts,repo,action,nevra) VALUES(?1,?,'add',?)",
                params![now, repo, nevra],
            )?;
        }
        // A rebuild refreshes the catalog columns of pre-existing packages.
        for p in &backfill {
            let nevra = p.nevra();
            tx.execute(
                "UPDATE packages SET name=?,epoch=?,version=?,release=?,arch=?,license=?,vendor=?,packager=?,url=?,summary=?,sourcerpm=?,build_time=?,size_package=?,size_installed=? WHERE repo=? AND nevra=?",
                params![
                    p.name(), p.epoch(), p.version(), p.release(), p.arch(),
                    p.rpm_license(), p.rpm_vendor(), p.packager(), p.url(), p.summary(), p.rpm_sourcerpm(),
                    p.time_build() as i64, p.size_package() as i64, p.size_installed() as i64,
                    repo, nevra,
                ],
            )?;
            tx.execute(
                "DELETE FROM requires WHERE repo=? AND nevra=?",
                params![repo, &nevra],
            )?;
            tx.execute(
                "DELETE FROM provides WHERE repo=? AND nevra=?",
                params![repo, &nevra],
            )?;
            tx.execute(
                "DELETE FROM files WHERE repo=? AND nevra=?",
                params![repo, &nevra],
            )?;
            write_dependencies(&tx, repo, &nevra, "requires", p.requires())?;
            write_dependencies(&tx, repo, &nevra, "provides", p.provides())?;
            write_files(&tx, repo, &nevra, p)?;
        }
        if let Some(ref rev) = revision {
            tx.execute("UPDATE repos SET revision=? WHERE id=?", params![rev, repo])?;
            log::info!("published {repo}: {} new RPMs, revision {rev}", added.len());
        }
        for id in &done {
            tx.execute(
                "UPDATE jobs SET status='published',detail='' WHERE id=?",
                [id],
            )?;
        }
        tx.commit()?;
        for href in removed.values() {
            fs::remove_file(base.join(href))?;
        }
        if let Some(current) = revision {
            // Keep cached manifests usable across publication boundaries.
            for entry in fs::read_dir(base.join("md"))?.flatten() {
                if entry
                    .file_name()
                    .to_str()
                    .is_some_and(|v| v != current && Uuid::parse_str(v).is_ok())
                    && entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(7 * 86400))
                    && let Err(e) = fs::remove_dir_all(entry.path())
                {
                    log::warn!("retire old metadata: {e}");
                }
            }
        }
        for id in done {
            let spool = base.join("queue").join(id);
            if spool.exists() {
                fs::remove_file(spool)?;
            }
        }
        Ok(())
    }
}
