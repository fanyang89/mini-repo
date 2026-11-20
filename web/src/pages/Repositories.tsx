import { useEffect, useMemo, useRef, useState } from "react"
import {
  AlertCircle,
  Check,
  Copy,
  Package,
  PackagePlus,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Upload,
} from "lucide-react"
import { toast } from "sonner"
import { EmptyState } from "@/components/EmptyState"
import { JobsPanel } from "@/components/JobsPanel"
import { StatusDot } from "@/components/StatusBadge"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  type Entry,
  type RepoInfo,
  createRepo,
  deleteRepo,
  deleteRpm,
  listPackages,
  listRepos,
  rebuildRepo,
  rootUrl,
  updateRepo,
} from "@/lib/api"
import { cn } from "@/lib/utils"
import { useRepoJobs } from "@/lib/useRepoJobs"

const EMPTY_FORM = { repo: "", name: "", arch: "" }
const PLACEHOLDERS = { repo: "el9-x86_64", name: "EL 9 packages for x86_64" }
const ARCH_OPTIONS = ["x86_64", "aarch64"]

// Preset choices; a repository created via the API with another architecture
// still shows its current value as an extra option.
function ArchSelect({ id, value, onChange }: { id: string; value: string; onChange: (v: string) => void }) {
  const options = value && !ARCH_OPTIONS.includes(value) ? [...ARCH_OPTIONS, value] : ARCH_OPTIONS
  return (
    <Select value={value} onValueChange={onChange}>
      <SelectTrigger id={id} className="w-full">
        <SelectValue placeholder="x86_64" />
      </SelectTrigger>
      <SelectContent>
        {options.map((a) => (
          <SelectItem key={a} value={a}>{a}</SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function act(action: () => Promise<unknown>, message: string) {
  return action()
    .then(() => toast.success(message))
    .catch((e: unknown) => toast.error(e instanceof Error ? e.message : String(e)))
}

// Local editable fields seeded from the repository; remounts on repo change.
function EditForm({ info, onSaved }: { info: RepoInfo; onSaved: () => Promise<unknown> }) {
  const [form, setForm] = useState({ name: info.repo.name, arch: info.repo.arch })
  return (
    <>
      <div className="grid gap-1">
        <Label htmlFor="edit-name">Name</Label>
        <Input
          id="edit-name"
          value={form.name}
          placeholder={info.repo.id}
          onChange={(e) => setForm({ ...form, name: e.target.value })}
        />
      </div>
      <div className="grid gap-1">
        <Label htmlFor="edit-arch">Architecture</Label>
        <ArchSelect id="edit-arch" value={form.arch} onChange={(v) => setForm({ ...form, arch: v })} />
      </div>
      <p className="col-span-full text-xs text-muted-foreground">
        The ID stays fixed so existing <code>baseurl</code>s keep working. Architecture is locked
        while the repository holds packages.
      </p>
      <Button
        onClick={() =>
          act(async () => {
            await updateRepo({ repo: info.repo.id, ...form })
            await onSaved()
          }, "Repository updated.")
        }
      >
        Save changes
      </Button>
    </>
  )
}

function Chip({ active, children, onClick }: { active: boolean; children: React.ReactNode; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "rounded-full border px-2.5 py-0.5 text-xs transition-colors",
        active
          ? "border-primary bg-primary text-primary-foreground"
          : "text-muted-foreground hover:bg-accent",
      )}
    >
      {children}
    </button>
  )
}

export function Repositories() {
  const [repos, setRepos] = useState<RepoInfo[] | null>(null)
  const [arch, setArch] = useState("")
  const [selectedId, setSelectedId] = useState("")
  const [query, setQuery] = useState("")
  const [all, setAll] = useState(false)
  const [rows, setRows] = useState<{ repo: string; entry: Entry; deletable: boolean }[]>([])
  const [searched, setSearched] = useState(false)
  const [more, setMore] = useState(false)
  const [createOpen, setCreateOpen] = useState(false)
  const [createForm, setCreateForm] = useState(EMPTY_FORM)
  const [deleteRepoOpen, setDeleteRepoOpen] = useState(false)
  const [deleteConfirmText, setDeleteConfirmText] = useState("")
  const [deleteTarget, setDeleteTarget] = useState<{ repo: string; nevra: string; href: string } | null>(null)
  const offset = useRef(0)
  const pendingDeletes = useRef(new Set<string>())
  const settledSeen = useRef(new Set<string>())

  const filtered = (repos ?? []).filter((r) => !arch || r.repo.arch === arch)
  // Filters can invalidate the stored selection; fall back to the first match.
  const selected = filtered.some((r) => r.repo.id === selectedId)
    ? selectedId
    : (filtered[0]?.repo.id ?? "")
  const current = (repos ?? []).find((r) => r.repo.id === selected)
  const jobs = useRepoJobs(selected, current?.write === true)
  const uploadHref = `${new URL("mini-repo/upload", rootUrl).pathname}?repo=${selected}`

  const search = async (reset: boolean) => {
    try {
      if (reset) {
        offset.current = 0
        setRows([])
      }
      const visible = filtered.filter((r) => r.read && (all || r.repo.id === selected))
      let hasMore = false
      const next: typeof rows = []
      for (const r of visible) {
        const entries = await listPackages(r.repo.id, query, offset.current)
        hasMore ||= entries.length === 100
        next.push(...entries.map((entry) => ({ repo: r.repo.id, entry, deletable: r.delete })))
      }
      offset.current += 100
      setRows(next)
      setMore(hasMore)
      setSearched(true)
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  const reload = async (keep?: string) => {
    const list = await listRepos()
    setRepos(list)
    const ids = list.map((r) => r.repo.id)
    if (keep !== undefined && ids.includes(keep)) setSelectedId(keep)
    else setSelectedId((prev: string) => (ids.includes(prev) ? prev : (ids[0] ?? "")))
  }

  useEffect(() => {
    const initial = new URLSearchParams(location.search).get("repo") ?? ""
    reload(initial || undefined)
  }, [])

  useEffect(() => {
    if (selected) search(true)
  }, [selected, query, all]) // eslint-disable-line react-hooks/exhaustive-deps

  // Refresh the table whenever a job settles: a publish may land after a
  // search raced it, and deletions must clear their row.
  useEffect(() => {
    const settled = jobs.jobs.filter(
      (j) => !settledSeen.current.has(j.id) && (j.status === "published" || j.status === "failed"),
    )
    if (settled.length === 0) return
    for (const j of settled) {
      settledSeen.current.add(j.id)
      if (pendingDeletes.current.delete(j.id)) {
        if (j.status === "failed") toast.error(`RPM deletion failed: ${j.detail}`)
        else toast.success("RPM deleted.")
      }
    }
    search(true)
  }, [jobs.jobs]) // eslint-disable-line react-hooks/exhaustive-deps

  // "/" focuses package search.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName ?? ""
      if (e.key === "/" && !/input|textarea|select/i.test(tag)) {
        e.preventDefault()
        document.getElementById("package-search")?.focus()
      }
    }
    addEventListener("keydown", onKey)
    return () => removeEventListener("keydown", onKey)
  }, [])

  const archs = useMemo(() => [...new Set((repos ?? []).map((r) => r.repo.arch))], [repos])
  const config = current
    ? `[${current.repo.id}]\nname=${current.repo.name || current.repo.id}\nbaseurl=${new URL(`repos/${current.repo.id}/`, rootUrl)}\nenabled=1`
    : ""
  const [copied, setCopied] = useState(false)

  if (repos === null) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-10 w-72" />
        <div className="grid gap-6 lg:grid-cols-[280px_1fr]">
          <Skeleton className="h-64" />
          <Skeleton className="h-72" />
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create repository</DialogTitle>
            <DialogDescription>
              The ID is the stable DNF base URL; the name is a human label shown
              in the <code>name=</code> field. One repository per architecture.
            </DialogDescription>
          </DialogHeader>
          <div className="grid gap-3">
            <div className="grid gap-1">
              <Label htmlFor="repo">ID</Label>
              <Input
                id="repo"
                value={createForm.repo}
                placeholder={PLACEHOLDERS.repo}
                onChange={(e) => setCreateForm({ ...createForm, repo: e.target.value })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                value={createForm.name}
                placeholder={PLACEHOLDERS.name}
                onChange={(e) => setCreateForm({ ...createForm, name: e.target.value })}
              />
            </div>
            <div className="grid gap-1">
              <Label htmlFor="arch">Architecture</Label>
              <ArchSelect
                id="arch"
                value={createForm.arch}
                onChange={(v) => setCreateForm({ ...createForm, arch: v })}
              />
            </div>
          </div>
          <DialogFooter>
            <Button
              disabled={Object.values(createForm).some((v) => !v)}
              onClick={async () => {
                await act(async () => {
                  await createRepo(createForm)
                  await reload(createForm.repo)
                  setCreateForm(EMPTY_FORM)
                  setCreateOpen(false)
                }, "Repository created.")
              }}
            >
              Create
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {repos.length === 0 ? (
        <EmptyState
          icon={PackagePlus}
          title="No repositories yet"
          description="Create your first repository, then upload RPMs — each one gets a stable DNF base URL."
          action={
            <Button onClick={() => setCreateOpen(true)}>
              <Plus /> Create repository
            </Button>
          }
        />
      ) : (
        <div className="grid items-start gap-6 lg:grid-cols-[280px_1fr]">
          <aside className="space-y-3">
            <Button className="w-full" onClick={() => setCreateOpen(true)}>
              <Plus /> New repository
            </Button>
            <div className="space-y-1.5 text-xs">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-muted-foreground">Architecture:</span>
                <Chip active={!arch} onClick={() => setArch("")}>All</Chip>
                {archs.map((a) => (
                  <Chip key={a} active={arch === a} onClick={() => setArch(a)}>{a}</Chip>
                ))}
              </div>
            </div>
            <div className="overflow-hidden rounded-lg border">
              {filtered.map((info) => (
                <button
                  key={info.repo.id}
                  onClick={() => setSelectedId(info.repo.id)}
                  className={cn(
                    "flex w-full items-center gap-2.5 border-b px-3 py-2.5 text-left text-sm last:border-b-0",
                    selected === info.repo.id ? "bg-accent" : "hover:bg-accent/50",
                  )}
                >
                  <StatusDot on={info.repo.revision !== null} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{info.repo.id}</span>
                    <span className="block truncate text-xs text-muted-foreground">
                      {[info.repo.name, info.repo.arch].filter(Boolean).join(" · ")}
                    </span>
                  </span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {info.repo.packages} RPM{info.repo.packages === 1 ? "" : "s"}
                  </span>
                </button>
              ))}
              {filtered.length === 0 && (
                <p className="p-4 text-sm text-muted-foreground">
                  No repository matches these filters.
                </p>
              )}
            </div>
          </aside>

          {current ? (
            <section className="min-w-0 space-y-4">
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <h2 className="text-xl font-semibold">{current.repo.id}</h2>
                <Badge variant={current.repo.revision ? "secondary" : "outline"}>
                  {current.repo.revision ? "published" : "empty"}
                </Badge>
                <span className="text-sm text-muted-foreground">
                  {current.repo.name || current.repo.id} · {current.repo.arch} ·{" "}
                  {current.repo.packages} RPM{current.repo.packages === 1 ? "" : "s"}
                </span>
                <Button variant="outline" size="sm" className="ml-auto" asChild>
                  <a href={uploadHref}>
                    <Upload /> Upload here
                  </a>
                </Button>
              </div>

              <Tabs defaultValue="packages">
                <TabsList>
                  <TabsTrigger value="packages">Packages</TabsTrigger>
                  <TabsTrigger value="jobs">Jobs</TabsTrigger>
                  {current.manage && <TabsTrigger value="settings">Settings</TabsTrigger>}
                  <TabsTrigger value="dnf">DNF</TabsTrigger>
                </TabsList>

                <TabsContent value="packages" className="space-y-3">
                  <form
                    className="flex flex-wrap items-center gap-3"
                    onSubmit={(e) => {
                      e.preventDefault()
                      search(true)
                    }}
                  >
                    <div className="relative min-w-56 flex-1">
                      <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
                      <Input
                        id="package-search"
                        className="pl-8"
                        placeholder="Search packages…"
                        value={query}
                        onChange={(e) => setQuery(e.target.value)}
                      />
                    </div>
                    <label className="flex items-center gap-2 text-sm text-muted-foreground">
                      <input type="checkbox" checked={all} onChange={(e) => setAll(e.target.checked)} />
                      All readable repositories
                    </label>
                    <Button type="submit" variant="secondary">Search</Button>
                  </form>
                  {rows.length > 0 ? (
                    <>
                      <div className="overflow-hidden rounded-lg border">
                        <Table>
                          <TableHeader>
                            <TableRow>
                              <TableHead>Repository</TableHead>
                              <TableHead>Package</TableHead>
                              <TableHead className="w-0" />
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {rows.map(({ repo, entry, deletable }) => (
                              <TableRow key={`${repo}/${entry.href}`}>
                                <TableCell className="text-muted-foreground">{repo}</TableCell>
                                <TableCell className="min-w-0">
                                  <a
                                    className="underline decoration-muted-foreground/40 underline-offset-2 hover:decoration-current"
                                    href={new URL(`repos/${repo}/${entry.href}`, rootUrl).href}
                                  >
                                    {entry.nevra}
                                  </a>
                                </TableCell>
                                <TableCell>
                                  {deletable && (
                                    <Button
                                      size="sm"
                                      variant="ghost"
                                      onClick={() =>
                                        setDeleteTarget({ repo, nevra: entry.nevra, href: entry.href })
                                      }
                                    >
                                      <Trash2 /> Delete
                                    </Button>
                                  )}
                                </TableCell>
                              </TableRow>
                            ))}
                          </TableBody>
                        </Table>
                      </div>
                      {more && (
                        <Button variant="outline" onClick={() => search(false)}>
                          More results
                        </Button>
                      )}
                    </>
                  ) : searched ? (
                    <EmptyState
                      icon={Search}
                      title="No matching package"
                      description={query ? `Nothing matches “${query}”.` : undefined}
                    />
                  ) : current.repo.packages === 0 ? (
                    <EmptyState
                      icon={Upload}
                      title="No packages yet"
                      description="Upload RPMs and they appear here as soon as the batch is published."
                      action={
                        <Button asChild>
                          <a href={uploadHref}>
                            <Upload /> Upload RPMs
                          </a>
                        </Button>
                      }
                    />
                  ) : (
                    <div className="space-y-2">
                      <Skeleton className="h-10" />
                      <Skeleton className="h-10" />
                      <Skeleton className="h-10" />
                    </div>
                  )}
                </TabsContent>

                <TabsContent value="jobs">
                  <JobsPanel repo={selected} jobs={jobs.jobs} partials={jobs.partials} refresh={jobs.refresh} />
                </TabsContent>

                {current.manage && (
                  <TabsContent value="settings" className="space-y-4">
                    <div className="grid gap-3 rounded-lg border p-4 sm:grid-cols-3">
                      <EditForm key={current.repo.id} info={current} onSaved={() => reload(current.repo.id)} />
                    </div>
                    <div className="flex flex-wrap gap-2 rounded-lg border p-4">
                      <Button
                        variant="outline"
                        onClick={() => act(() => rebuildRepo(current.repo.id), "Full rebuild queued.")}
                      >
                        <RefreshCw /> Rebuild index
                      </Button>
                      <p className="w-full text-xs text-muted-foreground">
                        Rereads every stored RPM and rewrites the index. Only needed to verify
                        storage integrity.
                      </p>
                    </div>
                    <div className="flex flex-wrap items-center gap-2 rounded-lg border border-red-500/40 p-4">
                      <div className="min-w-0 flex-1">
                        <p className="font-medium">Delete this repository</p>
                        <p className="text-xs text-muted-foreground">
                          Removes all {current.repo.packages} package
                          {current.repo.packages === 1 ? "" : "s"}, partial uploads and jobs. DNF
                          clients lose it immediately.
                        </p>
                      </div>
                      {current.delete && (
                        <Dialog open={deleteRepoOpen} onOpenChange={setDeleteRepoOpen}>
                          <DialogTrigger asChild>
                            <Button variant="destructive">
                              <Trash2 /> Delete repository
                            </Button>
                          </DialogTrigger>
                          <DialogContent>
                            <DialogHeader>
                              <DialogTitle>Delete {current.repo.id}?</DialogTitle>
                              <DialogDescription>
                                This cannot be undone. Type the repository ID to confirm.
                              </DialogDescription>
                            </DialogHeader>
                            <Input
                              placeholder={current.repo.id}
                              value={deleteConfirmText}
                              onChange={(e) => setDeleteConfirmText(e.target.value)}
                            />
                            <DialogFooter>
                              <Button
                                variant="destructive"
                                disabled={deleteConfirmText !== current.repo.id}
                                onClick={async () => {
                                  await act(async () => {
                                    await deleteRepo(current.repo.id)
                                    await reload()
                                    setDeleteRepoOpen(false)
                                    setDeleteConfirmText("")
                                  }, "Repository deleted.")
                                }}
                              >
                                Delete
                              </Button>
                            </DialogFooter>
                          </DialogContent>
                        </Dialog>
                      )}
                    </div>
                  </TabsContent>
                )}

                <TabsContent value="dnf" className="space-y-3">
                  <div className="rounded-lg border">
                    <div className="flex items-center justify-between border-b px-4 py-2">
                      <span className="text-sm font-medium">.repo stanza</span>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={async () => {
                          await navigator.clipboard.writeText(config)
                          setCopied(true)
                          setTimeout(() => setCopied(false), 1500)
                        }}
                      >
                        {copied ? <Check /> : <Copy />}
                        {copied ? "Copied" : "Copy"}
                      </Button>
                    </div>
                    <pre className="overflow-x-auto p-4 text-sm whitespace-pre">{config}</pre>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    Save to <code>/etc/yum.repos.d/{current.repo.id}.repo</code> on the client.
                  </p>
                </TabsContent>
              </Tabs>
            </section>
          ) : (
            <EmptyState
              icon={Package}
              title="Select a repository"
              description="Pick one on the left, or clear the filters to see all repositories."
            />
          )}
        </div>
      )}

      <Dialog open={deleteTarget !== null} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {deleteTarget?.nevra}?</DialogTitle>
            <DialogDescription>
              Removes the RPM from {deleteTarget?.repo} and publishes a new index. Already-running
              DNF clients keep working from the previous metadata.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="destructive"
              onClick={async () => {
                if (!deleteTarget) return
                const target = deleteTarget
                setDeleteTarget(null)
                await act(async () => {
                  const { id } = await deleteRpm(target.repo, target.href)
                  pendingDeletes.current.add(id)
                }, "RPM deletion queued.")
              }}
            >
              <AlertCircle /> Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
