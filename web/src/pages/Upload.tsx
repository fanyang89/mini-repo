import { useEffect, useRef, useState } from "react"
import {
  AlertCircle,
  CheckCircle2,
  FileUp,
  Loader2,
  UploadCloud,
} from "lucide-react"
import { toast } from "sonner"
import { JobsPanel } from "@/components/JobsPanel"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  type RepoInfo,
  listRepos,
  submitBatch,
  uploadFile,
} from "@/lib/api"
import { cn } from "@/lib/utils"
import { useRepoJobs } from "@/lib/useRepoJobs"

type Phase = "pending" | "uploading" | "received" | "failed"

interface Transfer {
  name: string
  size: number
  progress: number
  phase: Phase
  error?: string
}

export function Upload() {
  const [repos, setRepos] = useState<RepoInfo[]>([])
  const [selected, setSelected] = useState("")
  const [files, setFiles] = useState<FileList | null>(null)
  const [transfers, setTransfers] = useState<Transfer[]>([])
  const [uploading, setUploading] = useState(false)
  const [dragging, setDragging] = useState(false)
  const dropRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  const writable = repos.filter((r) => r.write)
  const current = writable.find((r) => r.repo.id === selected)
  const jobs = useRepoJobs(selected, current?.write === true, uploading)

  useEffect(() => {
    listRepos().then((list) => {
      setRepos(list)
      const wanted = new URLSearchParams(location.search).get("repo") ?? ""
      setSelected(list.some((r) => r.repo.id === wanted && r.write) ? wanted : "")
    })
  }, [])

  const start = async () => {
    if (uploading || !selected || !files || files.length === 0) return
    const batch = crypto.randomUUID()
    const list = [...files]
    setUploading(true)
    setTransfers(list.map((f) => ({ name: f.name, size: f.size, progress: 0, phase: "pending" as Phase })))
    try {
      for (const [i, file] of list.entries()) {
        const patch = (updater: (t: Transfer) => Transfer) =>
          setTransfers((prev) => prev.map((t, j) => (j === i ? updater(t) : t)))
        try {
          patch((t) => ({ ...t, phase: "uploading" }))
          await uploadFile(selected, file, batch, (fraction) =>
            patch((t) => ({ ...t, progress: fraction })),
          )
          patch((t) => ({ ...t, phase: "received", progress: 1 }))
        } catch (e) {
          patch((t) => ({
            ...t,
            phase: "failed",
            error: e instanceof Error ? e.message : String(e),
          }))
        }
      }
      await submitBatch(selected, batch)
      toast.success("Batch submitted — publishing now.")
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    } finally {
      setUploading(false)
      jobs.refresh()
    }
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6">
      <div className="space-y-1">
        <h1 className="text-xl font-semibold">Upload RPMs</h1>
        <p className="text-sm text-muted-foreground">
          One batch, one publication — everything that arrives is indexed together.
        </p>
      </div>

      <div className="space-y-1">
        <Label>Target repository</Label>
        <Select value={selected} onValueChange={setSelected} disabled={uploading}>
          <SelectTrigger className="w-full">
            <SelectValue
              placeholder={writable.length === 0 ? "No writable repository" : "Choose a repository"}
            />
          </SelectTrigger>
          <SelectContent>
            {writable.map((r) => (
              <SelectItem key={r.repo.id} value={r.repo.id}>
                {r.repo.name ? `${r.repo.name} · ` : ""}{r.repo.arch} — {r.repo.id}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <Card>
        <CardContent className="pt-6">
          <div
            ref={dropRef}
            role="button"
            tabIndex={0}
            className={cn(
              "flex cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed p-10 text-center transition-colors",
              dragging ? "border-primary bg-accent" : "hover:bg-accent/40",
            )}
            onClick={() => !uploading && inputRef.current?.click()}
            onKeyDown={(e) => e.key === "Enter" && !uploading && inputRef.current?.click()}
            onDragOver={(e) => e.preventDefault()}
            onDragEnter={() => setDragging(true)}
            onDragLeave={() => setDragging(false)}
            onDrop={(e) => {
              e.preventDefault()
              setDragging(false)
              if (!uploading) setFiles(e.dataTransfer.files)
            }}
          >
            <FileUp className="size-8 text-muted-foreground" />
            <p className="font-medium">
              {files && files.length > 0
                ? `${files.length} file${files.length === 1 ? "" : "s"} selected`
                : "Drop RPMs here or browse"}
            </p>
            <p className="text-xs text-muted-foreground">
              Interrupted transfers resume automatically — select the same file again.
            </p>
            <input
              ref={inputRef}
              type="file"
              accept=".rpm"
              multiple
              className="hidden"
              disabled={uploading}
              onChange={(e) => setFiles(e.target.files)}
            />
          </div>
          {transfers.length > 0 && (
            <ul className="mt-4 space-y-3">
              {transfers.map((t) => (
                <li key={t.name} className="space-y-1.5 text-sm">
                  <div className="flex items-center gap-2">
                    {t.phase === "received" && <CheckCircle2 className="size-4 shrink-0 text-emerald-500" />}
                    {t.phase === "failed" && <AlertCircle className="size-4 shrink-0 text-red-500" />}
                    {t.phase === "uploading" && <Loader2 className="size-4 shrink-0 animate-spin text-muted-foreground" />}
                    <span className="min-w-0 flex-1 break-all">{t.name}</span>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {t.phase === "uploading" && `${Math.round(t.progress * 100)}%`}
                      {t.phase === "received" && "received"}
                      {t.phase === "pending" && "waiting"}
                      {t.phase === "failed" && "interrupted"}
                    </span>
                  </div>
                  {t.phase === "uploading" && <Progress value={t.progress * 100} />}
                  {t.phase === "failed" && (
                    <p className="text-xs text-muted-foreground">
                      {t.error} Select the same file again to resume.
                    </p>
                  )}
                </li>
              ))}
            </ul>
          )}
          <Button
            size="lg"
            className="mt-4 w-full"
            disabled={uploading || !selected || !files || files.length === 0}
            onClick={() => start()}
          >
            <UploadCloud />
            {uploading ? "Uploading…" : "Upload batch"}
          </Button>
        </CardContent>
      </Card>

      {current && (
        <JobsPanel repo={selected} jobs={jobs.jobs} partials={jobs.partials} refresh={jobs.refresh} />
      )}
    </div>
  )
}
