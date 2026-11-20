// Build the Vite SPA when its sources change, so the crate can embed web/dist.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn main() {
    let web: PathBuf = ["..", "web"].iter().collect();
    let dist = web.join("dist");
    let stamp = dist.join(".build-stamp");
    let inputs = [
        web.join("src"),
        web.join("index.html"),
        web.join("package.json"),
        web.join("pnpm-lock.yaml"),
        web.join("vite.config.ts"),
        web.join("tsconfig.json"),
        web.join("tsconfig.app.json"),
        web.join("components.json"),
    ];
    for path in &inputs {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let stale =
        |path: &Path| newest(path).is_some_and(|t| stamp_mtime(&stamp).is_none_or(|s| t > s));
    if !inputs.iter().any(|p| stale(p)) {
        return;
    }
    if !web.join("node_modules").exists() {
        run(&web, &["install", "--frozen-lockfile"]);
    }
    run(&web, &["build"]);
    fs::write(&stamp, b"").expect("write build stamp");
}

fn run(web: &Path, args: &[&str]) {
    let status = Command::new("pnpm").args(args).current_dir(web).status();
    let status = match status {
        Ok(s) => s,
        Err(_) => panic!(
            "pnpm not found; the SPA is required: run `cd web && pnpm install && pnpm build`"
        ),
    };
    if !status.success() {
        panic!("pnpm {} failed", args.join(" "));
    }
}

fn stamp_mtime(stamp: &Path) -> Option<SystemTime> {
    fs::metadata(stamp).and_then(|m| m.modified()).ok()
}

fn newest(path: &Path) -> Option<SystemTime> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_dir() {
        return meta.modified().ok();
    }
    fs::read_dir(path)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| newest(&e.path()))
        .max()
}
