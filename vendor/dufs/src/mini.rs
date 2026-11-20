use crate::{
    http_utils::body_full,
    server::{Request, Response, Server},
};
use anyhow::Result;
use bytes::Bytes;
use hyper::{Method, StatusCode};
use mini_repo_core::{component, Store};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

pub fn reserved(path: &str) -> bool {
    matches!(
        path.split('/').next(),
        Some("" | "mini-repo" | "repos" | "incoming")
    )
}
pub fn incoming(path: &str) -> Option<(&str, &str)> {
    let mut p = path.split('/');
    match (p.next(), p.next(), p.next(), p.next()) {
        (Some("incoming"), Some(r), Some(f), None)
            if component(r) && component(f) && f.ends_with(".rpm") =>
        {
            Some((r, f))
        }
        _ => None,
    }
}
fn reply(status: StatusCode, typ: &str, body: impl Into<Bytes>) -> Response {
    hyper::Response::builder()
        .status(status)
        .header("content-type", typ)
        .header("cache-control", "no-store")
        .header("x-content-type-options", "nosniff")
        .body(body_full(body))
        .unwrap()
}
fn message(status: StatusCode, msg: &str) -> Response {
    reply(status, "text/plain; charset=utf-8", msg.to_owned())
}
fn data(value: Value) -> Response {
    reply(StatusCode::OK, "application/json", value.to_string())
}
fn header<'a>(req: &'a Request, key: &str) -> Option<&'a str> {
    req.headers().get(key)?.to_str().ok()
}

// Serve the embedded Vite SPA: hashed assets are immutable, everything else
// gets the index.html history fallback.
fn spa(relative_path: &str, head_only: bool) -> Response {
    let file = relative_path
        .strip_prefix("mini-repo/")
        .and_then(mini_repo_core::spa_file);
    match file {
        Some(bytes) if head_only => hyper::Response::builder()
            .header("content-type", asset_type(relative_path))
            .header("cache-control", "public, max-age=31536000, immutable")
            .header("content-length", bytes.len())
            .body(body_full(Bytes::new()))
            .unwrap(),
        Some(bytes) => hyper::Response::builder()
            .header("content-type", asset_type(relative_path))
            .header("cache-control", "public, max-age=31536000, immutable")
            .body(body_full(bytes))
            .unwrap(),
        None if head_only => hyper::Response::builder()
            .header("content-type", "text/html; charset=utf-8")
            .header("cache-control", "no-store")
            .body(body_full(Bytes::new()))
            .unwrap(),
        None => reply(
            StatusCode::OK,
            "text/html; charset=utf-8",
            mini_repo_core::spa_file("index.html").unwrap_or_default(),
        ),
    }
}

fn asset_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("json" | "map") => "application/json",
        _ => "application/octet-stream",
    }
}

impl Server {
    pub(crate) async fn mini_route(
        &self,
        store: &Store,
        path: &str,
        req: &Request,
        user: Option<&str>,
        token: bool,
        q: &HashMap<String, String>,
    ) -> Result<Option<Response>> {
        let parts: Vec<_> = path.split('/').collect();
        let method = req.method();
        let deny = || Some(message(StatusCode::FORBIDDEN, "Forbidden"));
        let missing = || Some(message(StatusCode::NOT_FOUND, "Not found"));
        let get = method == Method::GET;
        let auth = &self.args.auth;
        let root = &self.args.serve_path;
        let arg = |key: &str| q.get(key).map_or("", String::as_str);
        match parts[0] {
            "repos" => {
                if !(get || method == Method::HEAD) {
                    return Ok(deny());
                }
                if parts.len() < 4 {
                    return Ok(missing());
                }
                if let Some(file) = store
                    .download(parts[1], &parts[2..])?
                    .filter(|f| f.is_file())
                {
                    let mut res = Response::default();
                    self.handle_send_file(&file, req.headers(), method == Method::HEAD, &mut res)
                        .await?;
                    return Ok(Some(res));
                }
                Ok(missing())
            }
            "mini-repo" => {
                if token {
                    return Ok(deny());
                }
                if parts.get(1) != Some(&"api") {
                    if !(get || method == Method::HEAD) {
                        return Ok(deny());
                    }
                    return Ok(Some(spa(path, method == Method::HEAD)));
                }
                if !get
                    && !(method == Method::POST
                        && header(req, "x-mini-repo") == Some("1")
                        && self.args.allow_upload)
                {
                    return Ok(deny());
                }
                let r = arg("repo");
                if get && parts.get(2) == Some(&"repos") {
                    let mut list = vec![];
                    for repo in store.repos()? {
                        let write = self.args.allow_upload
                            && auth
                                .can_access(
                                    user,
                                    &format!("incoming/{}", repo.id),
                                    &Method::PUT,
                                    root,
                                )
                                .await;
                        let read = auth
                            .can_read(
                                user,
                                &format!("repos/{}/repodata/repomd.xml", repo.id),
                                root,
                            )
                            .await;
                        if read || write {
                            let manage = write
                                && auth.can_access(user, "incoming", &Method::PUT, root).await;
                            list.push(json!({"repo":repo,"read":read,"write":write,"manage":manage,"delete":write && self.args.allow_delete}));
                        }
                    }
                    return Ok(Some(data(json!(list))));
                }
                if !component(r) {
                    return Ok(Some(message(StatusCode::BAD_REQUEST, "Invalid repo ID")));
                }
                let writable = auth
                    .can_access(user, &format!("incoming/{r}"), &Method::PUT, root)
                    .await
                    && self.args.allow_upload;
                if method == Method::POST && parts.get(2) == Some(&"create") {
                    if !writable || !auth.can_access(user, "incoming", &Method::PUT, root).await {
                        return Ok(deny());
                    }
                    return Ok(Some(match store.create(r, arg("name"), arg("arch")) {
                        Ok(()) => data(json!({"created":r})),
                        Err(e) => message(StatusCode::BAD_REQUEST, &e.to_string()),
                    }));
                }
                if !store.exists(r)? {
                    return Ok(missing());
                }
                if get && parts.get(2) == Some(&"packages") {
                    if !auth
                        .can_read(user, &format!("repos/{r}/repodata/repomd.xml"), root)
                        .await
                    {
                        return Ok(deny());
                    }
                    return Ok(Some(data(json!(store.entries(
                        r,
                        arg("q"),
                        arg("offset").parse().unwrap_or(0)
                    )?))));
                }
                if get && parts.get(2) == Some(&"sbom") {
                    if !auth
                        .can_read(user, &format!("repos/{r}/repodata/repomd.xml"), root)
                        .await
                    {
                        return Ok(deny());
                    }
                    let host = header(req, "host").unwrap_or("localhost");
                    let scheme = if req.uri().scheme_str() == Some("https") {
                        "https"
                    } else {
                        "http"
                    };
                    let origin = format!("{scheme}://{host}");
                    let document = store.sbom(r, &origin, &format!("{origin}/repos/{r}/"))?;
                    return Ok(Some(
                        hyper::Response::builder()
                            .header("content-type", "application/json")
                            .header(
                                "content-disposition",
                                format!("attachment; filename=\"{r}-sbom.spdx.json\""),
                            )
                            .header("cache-control", "no-store")
                            .body(body_full(document))
                            .unwrap(),
                    ));
                }
                if !writable {
                    return Ok(deny());
                }
                // Repository-wide operations require write access throughout the inbox.
                if auth
                    .target_has_restricted_descendant(user, &format!("incoming/{r}"), root)
                    .await
                {
                    return Ok(deny());
                }
                Ok(Some(match (get, parts.get(2).copied()) {
                    (false, Some(action @ ("update" | "delete-repo"))) => {
                        if !auth.can_access(user, "incoming", &Method::PUT, root).await
                            || (action == "delete-repo" && !self.args.allow_delete)
                        {
                            return Ok(deny());
                        }
                        let result = if action == "delete-repo" {
                            store.delete_repo(r)
                        } else {
                            store.update(r, arg("name"), arg("arch"))
                        };
                        match result {
                            Ok(()) => data(json!(true)),
                            Err(e) => message(StatusCode::CONFLICT, &e.to_string()),
                        }
                    }
                    (false, Some("delete-rpm")) => {
                        if !self.args.allow_delete {
                            return Ok(deny());
                        }
                        match store.remove_package(r, arg("href")) {
                            Ok(id) => data(json!({"id":id})),
                            Err(e) => message(StatusCode::NOT_FOUND, &e.to_string()),
                        }
                    }
                    (false, Some("rebuild")) => {
                        store.rebuild(r)?;
                        data(json!(true))
                    }
                    (true, Some("jobs")) => {
                        data(json!({"jobs":store.jobs(r)?,"partials":store.partials(r)?}))
                    }
                    (false, Some("submit")) if component(arg("batch")) => {
                        store.submit(r, arg("batch"))?;
                        data(json!(true))
                    }
                    (false, Some("retry")) => match store.retry(r, arg("id")) {
                        Ok(()) => data(json!(true)),
                        Err(e) => message(StatusCode::BAD_REQUEST, &e.to_string()),
                    },
                    _ => return Ok(missing()),
                }))
            }
            _ => Ok(None),
        }
    }

    pub(crate) async fn mini_upload(
        &self,
        store: &Store,
        repo: &str,
        file: &str,
        req: Request,
    ) -> Result<Response> {
        let method = req.method().clone();
        let path = store.partial(repo, file);
        let mut hash = DefaultHasher::new();
        path.hash(&mut hash);
        let _lock = store.upload_locks[hash.finish() as usize % store.upload_locks.len()]
            .lock()
            .await;
        let size = tokio::fs::metadata(&path).await.ok().map(|m| m.len());
        if method == Method::HEAD {
            let mut res = Response::default();
            if let Some(n) = size {
                res.headers_mut().insert("content-length", n.into());
            } else {
                *res.status_mut() = StatusCode::NOT_FOUND
            }
            return Ok(res);
        }
        if !self.args.allow_upload
            || !(method == Method::PUT || method == Method::PATCH || method == Method::POST)
        {
            return Ok(message(
                StatusCode::FORBIDDEN,
                "Write via PUT or append PATCH",
            ));
        }
        let batch = header(&req, "x-mini-batch").unwrap_or("").to_string();
        if !batch.is_empty() && !component(&batch) {
            return Ok(message(StatusCode::BAD_REQUEST, "Invalid batch"));
        }
        let total = match header(&req, "x-mini-total")
            .map(str::parse::<u64>)
            .transpose()
        {
            Ok(t) => t,
            Err(_) => return Ok(message(StatusCode::BAD_REQUEST, "Invalid total size")),
        };
        if method == Method::POST {
            if header(&req, "x-mini-repo") != Some("1") || size.is_none() || total != size {
                return Ok(message(
                    StatusCode::CONFLICT,
                    "Confirm the exact complete file size",
                ));
            }
        } else {
            let offset = if method == Method::PATCH {
                if size.is_none() || header(&req, "x-update-range") != Some("append") {
                    return Ok(message(
                        StatusCode::CONFLICT,
                        "Resume with append and the current offset",
                    ));
                }
                if header(&req, "x-mini-offset")
                    .is_some_and(|start| start.parse::<u64>().ok() != size)
                {
                    return Ok(message(StatusCode::CONFLICT, "Partial size changed"));
                }
                size
            } else {
                if size.unwrap_or(0) > 0 && !self.args.allow_delete {
                    return Ok(message(StatusCode::CONFLICT, "Resume the existing upload"));
                }
                None
            };
            let mut res = Response::default();
            self.handle_upload(&path, offset, size.unwrap_or(0), req, &mut res)
                .await?;
            let written = tokio::fs::metadata(&path).await?.len();
            if total.is_some_and(|total| total != written) {
                return Ok(message(
                    StatusCode::CONFLICT,
                    "Upload incomplete; resume with PATCH",
                ));
            }
            if method == Method::PATCH && total.is_none() {
                return Ok(message(StatusCode::ACCEPTED, "Awaiting confirmation"));
            }
        }
        let id = store.enqueue(repo, file, &batch)?;
        Ok(reply(
            StatusCode::ACCEPTED,
            "application/json",
            json!({"id":id}).to_string(),
        ))
    }
}
