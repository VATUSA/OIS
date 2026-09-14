import {Link} from "@tanstack/react-router";
import {BookOpen, Code2, Radar} from "lucide-react";

import {DOCS_URL} from "@/lib/api";

const GITHUB_URL = "https://github.com/VATUSA/OIS";
const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;

// Full version like "1.0.1-a1b2c3d". The trailing segment is the build commit when it looks like a
// short SHA — link the stamp straight to that commit, else fall back to the repo.
const VERSION = __APP_VERSION__;
const COMMIT = /^[0-9a-f]{7,40}$/.test(VERSION.split("-").pop() ?? "")
  ? VERSION.split("-").pop()!
  : "";
const VERSION_HREF = COMMIT ? `${GITHUB_URL}/commit/${COMMIT}` : GITHUB_URL;

const linkClass = "transition-colors hover:text-foreground";

export function Footer() {
  const year = new Date().getFullYear();

  return (
    <footer className="border-t bg-background">
      <div className="mx-auto flex w-full max-w-7xl flex-col gap-4 px-4 py-6 text-sm text-muted-foreground sm:flex-row sm:items-center">
        <div className="flex items-center gap-2">
          <Radar className="size-4 text-primary" />
          <span className="font-medium text-foreground">OIS</span>
          <span className="hidden sm:inline">· VATUSA Events · Operational Information System</span>
        </div>

        <nav className="flex flex-wrap items-center gap-x-4 gap-y-2 sm:ml-auto">
          <Link to="/privacy" className={linkClass}>
            Privacy
          </Link>
          <a href={LICENSE_URL} target="_blank" rel="noreferrer" className={linkClass}>
            License · MIT
          </a>
          <a
            href={GITHUB_URL}
            target="_blank"
            rel="noreferrer"
            className={"inline-flex items-center gap-1.5 " + linkClass}
          >
            <Code2 className="size-3.5" />
            GitHub
          </a>
          {DOCS_URL && (
            <a
              href={DOCS_URL}
              target="_blank"
              rel="noreferrer"
              className={"inline-flex items-center gap-1.5 " + linkClass}
            >
              <BookOpen className="size-3.5" />
              Docs
            </a>
          )}
          <a
            href={VERSION_HREF}
            target="_blank"
            rel="noreferrer"
            className={"font-mono text-xs " + linkClass}
            title={COMMIT ? `Build commit ${COMMIT}` : "Source on GitHub"}
          >
            v{VERSION}
          </a>
        </nav>
      </div>

      <div className="mx-auto w-full max-w-7xl px-4 pb-6 text-xs text-muted-foreground">
        © {year} VATUSA. Not affiliated with any government or aviation authority — for use on the
        VATSIM network only.
      </div>
    </footer>
  );
}
