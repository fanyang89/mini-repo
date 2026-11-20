import { useCallback, useEffect, useRef, useState } from "react"
import { type Job, type JobsSnapshot, listJobs } from "@/lib/api"

const EMPTY: JobsSnapshot = { jobs: [], partials: [] }

// Poll a repository's job list every 5s; pause while hidden or uploading.
export function useRepoJobs(repo: string, enabled: boolean, paused = false): JobsSnapshot & { refresh: () => void } {
  const [result, setResult] = useState<{ repo: string; snapshot: JobsSnapshot }>()
  const request = useRef<symbol | null>(null)

  const refresh = useCallback(() => {
    if (!enabled) return
    const id = Symbol()
    request.current = id
    return listJobs(repo).then(snapshot => {
      if (id === request.current) setResult({ repo, snapshot })
    })
  }, [enabled, repo])

  useEffect(() => {
    if (!enabled) return
    refresh()
    const timer = setInterval(() => {
      if (!document.hidden && !paused) refresh()
    }, 5000)
    return () => {
      clearInterval(timer)
      request.current = null
    }
  }, [refresh, paused, enabled])

  return { ...(enabled && result?.repo === repo ? result.snapshot : EMPTY), refresh }
}

export type { Job }
