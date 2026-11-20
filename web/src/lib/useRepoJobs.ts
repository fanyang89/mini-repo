import { useCallback, useEffect, useState } from "react"
import { type Job, type JobsSnapshot, listJobs } from "@/lib/api"

// Poll a repository's job list every 5s; pause while hidden or uploading.
export function useRepoJobs(repo: string, enabled: boolean, paused = false): JobsSnapshot & { refresh: () => void } {
  const [snapshot, setSnapshot] = useState<JobsSnapshot>({ jobs: [], partials: [] })

  const refresh = useCallback(async () => {
    setSnapshot(!enabled ? { jobs: [], partials: [] } : await listJobs(repo))
  }, [enabled, repo])

  useEffect(() => {
    refresh()
    if (!enabled) return
    const timer = setInterval(() => {
      if (!document.hidden && !paused) refresh()
    }, 5000)
    return () => clearInterval(timer)
  }, [refresh, paused, enabled])

  return { ...snapshot, refresh }
}

export type { Job }
