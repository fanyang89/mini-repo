// Build the Vite SPA when its sources change, so the crate can embed web/dist.
use std::{path::Path, process::Command};

fn main() {
    let web = Path::new("../web");
    for input in [
        "src",
        "public",
        "index.html",
        "package.json",
        "pnpm-lock.yaml",
        "pnpm-workspace.yaml",
        "vite.config.ts",
        "tsconfig.json",
        "tsconfig.app.json",
        "tsconfig.node.json",
        "components.json",
    ] {
        println!("cargo:rerun-if-changed={}", web.join(input).display());
    }
    run(web, &["install", "--frozen-lockfile"]);
    run(web, &["build"]);
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
