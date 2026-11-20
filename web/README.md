# Mini Repo management UI

`src/pages/` contains the repository and upload screens. `src/lib/api.ts`
talks to the Rust service; `src/components/` holds shared UI.

Start the Rust server using the [root quickstart](../README.md#run). In a
second terminal, from `web/`, run:

```sh
pnpm install --frozen-lockfile
pnpm dev
```

Open the printed Vite URL at `/mini-repo/`. Vite forwards `/mini-repo/api`,
`/incoming` and `/repos` to `http://127.0.0.1:5000`. Set `MINI_REPO_BACKEND`
if the server listens elsewhere.

Use `pnpm lint` for lint checks. `pnpm build` type-checks and bundles the UI
into `dist/`; the root Cargo build runs this step and embeds the result.
To test the shipped UI in Chromium, use the [product tests](../README.md#verification).
