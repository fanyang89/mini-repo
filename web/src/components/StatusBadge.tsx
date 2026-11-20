import { Badge } from "@/components/ui/badge"
import { cn } from "@/lib/utils"

const JOBS: Record<string, string> = {
  queued: "border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-400",
  ready: "border-sky-500/40 bg-sky-500/10 text-sky-700 dark:text-sky-400",
  published: "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
  failed: "border-red-500/40 bg-red-500/10 text-red-700 dark:text-red-400",
}

export function StatusBadge({ status, className }: { status: string; className?: string }) {
  return (
    <Badge variant="outline" className={cn(JOBS[status] ?? "", className)}>
      {status}
    </Badge>
  )
}

export function StatusDot({ on }: { on: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "size-2 shrink-0 rounded-full",
        on ? "bg-emerald-500" : "bg-muted-foreground/40",
      )}
    />
  )
}
