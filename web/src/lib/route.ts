import { useEffect, useState } from "react"

// History routing for the two SPA views; no router dependency needed.
export function usePath(): string {
  const [path, setPath] = useState(location.pathname)
  useEffect(() => {
    const update = () => setPath(location.pathname)
    addEventListener("popstate", update)
    return () => removeEventListener("popstate", update)
  }, [])
  return path
}

export function navigate(to: string): void {
  history.pushState(null, "", to)
  dispatchEvent(new PopStateEvent("popstate"))
}
