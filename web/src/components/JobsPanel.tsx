import { useState } from "react"
import { FileWarning, RotateCw, Send } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { StatusBadge } from "@/components/StatusBadge"
import { confirmPartial, retryJob } from "@/lib/api"

export interface Job {
  id: string
  filename: string
  status: string
  detail: string
}

export interface JobsPanelProps {
  repo: string
  jobs: Job[]
  partials: string[]
  refresh: () => void
}

export function JobsPanel({ repo, jobs, partials, refresh }: JobsPanelProps) {
  const [confirming, setConfirming] = useState<string | null>(null)
  const [size, setSize] = useState("")

  const settle = async (action: () => Promise<unknown>, message: string) => {
    try {
      await action()
      toast.success(message)
      refresh()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Jobs</CardTitle>
        <CardDescription>Uploads, deletions and rebuilds for {repo}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {jobs.length === 0 && partials.length === 0 && (
          <p className="text-sm text-muted-foreground">Nothing yet — uploaded batches appear here.</p>
        )}
        <ul className="space-y-1.5">
          {jobs.map((j) => (
            <li key={j.id} className="flex flex-wrap items-center gap-2 text-sm">
              <StatusBadge status={j.status} />
              <span className="min-w-0 flex-1 break-all">{j.filename}</span>
              {j.detail && (
                <span className="min-w-0 flex-1 basis-full break-all pl-1 text-xs text-muted-foreground">
                  {j.detail}
                </span>
              )}
              {(j.status === "ready" || j.status === "failed") && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => settle(() => retryJob(repo, j.id), "Job queued.")}
                >
                  {j.status === "ready" ? <Send /> : <RotateCw />}
                  {j.status === "ready" ? "Publish" : "Retry"}
                </Button>
              )}
            </li>
          ))}
          {partials.map((f) => (
            <li key={f} className="flex flex-wrap items-center gap-2 text-sm">
              <FileWarning className="size-4 shrink-0 text-amber-500" aria-label="partial upload" />
              <span className="min-w-0 flex-1 break-all">{f}</span>
              <Button size="sm" variant="outline" onClick={() => setConfirming(f)}>
                Confirm completed file
              </Button>
            </li>
          ))}
        </ul>
        <Dialog open={confirming !== null} onOpenChange={(open) => !open && setConfirming(null)}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Confirm partial upload</DialogTitle>
              <DialogDescription>
                Enter the original size in bytes of {confirming}. The publisher rejects a wrong
                length, so a guess just fails safely.
              </DialogDescription>
            </DialogHeader>
            <div className="grid gap-2">
              <Label htmlFor="partial-size">Size in bytes</Label>
              <Input
                id="partial-size"
                inputMode="numeric"
                value={size}
                onChange={(e) => setSize(e.target.value)}
              />
            </div>
            <DialogFooter>
              <Button
                onClick={() =>
                  settle(async () => {
                    if (!confirming) return
                    await confirmPartial(repo, confirming, Number(size))
                    setConfirming(null)
                  }, "Partial submitted.")
                }
              >
                Confirm
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </CardContent>
    </Card>
  )
}
