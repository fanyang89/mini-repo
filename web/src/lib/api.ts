// Typed client for the mini-repo HTTP API served by the Rust binary.

export interface Repo {
  id: string
  name: string
  arch: string
  revision: string | null
  packages: number
}

export interface RepoInfo {
  repo: Repo
  read: boolean
  write: boolean
  manage: boolean
  delete: boolean
}

export interface Entry {
  nevra: string
  filename: string
  href: string
}

export interface Job {
  id: string
  filename: string
  status: string
  detail: string
}

export interface JobsSnapshot {
  jobs: Job[]
  partials: string[]
}

export interface RepoFields {
  repo: string
  name: string
  arch: string
}

const root = new URL(
  location.pathname.slice(0, location.pathname.indexOf("/mini-repo") + 1),
  location.href,
)
const base = new URL("mini-repo/", root)

export const rootUrl = root
export const repoUrl = (repo: Repo) => new URL(`repos/${repo.id}/`, root)

async function api<T>(name: string, params: object, post = false): Promise<T> {
  const u = new URL(`api/${name}`, base)
  u.search = new URLSearchParams(params as Record<string, string>).toString()
  const r = await fetch(u, {
    method: post ? "POST" : "GET",
    headers: post ? { "X-Mini-Repo": "1" } : {},
  })
  if (!r.ok) throw new Error(`${r.status}: ${await r.text()}`)
  return r.json()
}

export const listRepos = () => api<RepoInfo[]>("repos", {})
export const createRepo = (f: RepoFields) => api("create", f, true)
export const updateRepo = (f: RepoFields) => api("update", f, true)
export const deleteRepo = (repo: string) => api("delete-repo", { repo }, true)
export const rebuildRepo = (repo: string) => api("rebuild", { repo }, true)
export const submitBatch = (repo: string, batch: string) => api("submit", { repo, batch }, true)
export const retryJob = (repo: string, id: string) => api("retry", { repo, id }, true)
export const deleteRpm = (repo: string, href: string) => api<{ id: string }>("delete-rpm", { repo, href }, true)
export const listPackages = (repo: string, q: string, offset: number) =>
  api<Entry[]>("packages", { repo, q, offset: String(offset) })
export const listJobs = (repo: string) => api<JobsSnapshot>("jobs", { repo })

export async function confirmPartial(repo: string, file: string, total: number): Promise<void> {
  const u = new URL(`incoming/${repo}/${encodeURIComponent(file)}`, root)
  const r = await fetch(u, {
    method: "POST",
    headers: { "X-Mini-Repo": "1", "X-Mini-Total": String(total) },
  })
  if (!r.ok) throw new Error(await r.text())
}

// PUT the first chunk or PATCH-append the remainder of a resumable upload.
function send(
  repo: string,
  file: File,
  start: number,
  batch: string,
  onProgress: (fraction: number) => void,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest()
    x.open(start > 0 ? "PATCH" : "PUT", new URL(`incoming/${repo}/${encodeURIComponent(file.name)}`, root))
    x.setRequestHeader("X-Mini-Batch", batch)
    x.setRequestHeader("X-Mini-Total", String(file.size))
    if (start > 0) {
      x.setRequestHeader("X-Update-Range", "append")
      x.setRequestHeader("X-Mini-Offset", String(start))
    }
    x.upload.onprogress = (e) => onProgress((start + e.loaded) / file.size)
    x.onload = () =>
      x.status === 202 ? resolve() : reject(new Error(x.responseText || `HTTP ${x.status}`))
    x.onerror = () => reject(new Error("Transfer interrupted; select the same file to resume."))
    x.send(file.slice(start))
  })
}

export async function uploadFile(
  repo: string,
  file: File,
  batch: string,
  onProgress: (fraction: number) => void,
): Promise<void> {
  const u = new URL(`incoming/${repo}/${encodeURIComponent(file.name)}`, root)
  const h = await fetch(u, { method: "HEAD" })
  if (!h.ok && h.status !== 404) throw new Error(`HTTP ${h.status}`)
  const start = h.ok ? Number(h.headers.get("content-length")) : 0
  if (start > file.size) throw new Error("Existing partial is larger than this file.")
  await send(repo, file, start, batch, onProgress)
}
