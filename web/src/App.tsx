import { Package } from "lucide-react"
import { Toaster } from "@/components/ui/sonner"
import { Button } from "@/components/ui/button"
import { Repositories } from "@/pages/Repositories"
import { Upload } from "@/pages/Upload"
import { rootUrl } from "@/lib/api"
import { navigate, usePath } from "@/lib/route"
import { ThemeToggle } from "@/lib/theme"
import { cn } from "@/lib/utils"

const manageUrl = new URL("mini-repo/", rootUrl).pathname
const uploadUrl = new URL("mini-repo/upload", rootUrl).pathname

function NavLink({ to, children }: { to: string; children: React.ReactNode }) {
  const path = usePath()
  const active = to === manageUrl ? !path.endsWith("/upload") : path.endsWith("/upload")
  return (
    <button
      className={cn(
        "rounded-lg px-3 py-1.5 text-sm font-medium transition-colors",
        active ? "bg-accent text-accent-foreground" : "text-muted-foreground hover:bg-accent/60",
      )}
      onClick={() => navigate(to)}
    >
      {children}
    </button>
  )
}

export default function App() {
  const path = usePath()
  const upload = path.endsWith("/upload")
  return (
    <div className="min-h-svh bg-background text-foreground">
      <header className="sticky top-0 z-40 border-b bg-background/85 backdrop-blur">
        <div className="mx-auto flex h-14 max-w-6xl items-center gap-3 px-6">
          <button
            className="flex items-center gap-2 text-sm font-semibold"
            onClick={() => navigate(manageUrl)}
          >
            <Package className="size-4" aria-hidden />
            Mini Repo
          </button>
          <nav className="ml-2 flex items-center gap-1">
            <NavLink to={manageUrl}>Repositories</NavLink>
            <NavLink to={uploadUrl}>Upload</NavLink>
          </nav>
          <div className="ml-auto flex items-center gap-1">
            <ThemeToggle />
            <Button variant="ghost" size="sm" asChild>
              <a href={rootUrl.pathname} target="_blank" rel="noreferrer">
                File server ↗
              </a>
            </Button>
          </div>
        </div>
      </header>
      <main className="mx-auto max-w-6xl px-6 py-8">
        {upload ? <Upload /> : <Repositories />}
      </main>
      <Toaster position="top-center" richColors />
    </div>
  )
}
