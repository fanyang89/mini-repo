import { useEffect, useState } from "react"
import { Moon, Sun } from "lucide-react"
import { Button } from "@/components/ui/button"

type Theme = "light" | "dark"

function stored(): Theme | null {
  const v = localStorage.getItem("theme")
  return v === "light" || v === "dark" ? v : null
}
function system(): Theme {
  return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"
}
function apply(theme: Theme) {
  document.documentElement.classList.toggle("dark", theme === "dark")
}

// System preference by default; an explicit choice persists across visits.
export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(stored() ?? system())
  useEffect(() => {
    apply(stored() ?? system())
    const media = matchMedia("(prefers-color-scheme: dark)")
    const follow = () => {
      if (!stored()) {
        const next = system()
        setTheme(next)
        apply(next)
      }
    }
    media.addEventListener("change", follow)
    return () => media.removeEventListener("change", follow)
  }, [])
  return (
    <Button
      variant="ghost"
      size="icon"
      aria-label="Toggle theme"
      onClick={() => {
        const next: Theme = theme === "dark" ? "light" : "dark"
        localStorage.setItem("theme", next)
        setTheme(next)
        apply(next)
      }}
    >
      {theme === "dark" ? <Sun /> : <Moon />}
    </Button>
  )
}
