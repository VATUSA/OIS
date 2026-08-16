import {Link} from "@tanstack/react-router";
import {BookOpen, Code2, Radar} from "lucide-react";

import {DOCS_URL} from "@/lib/api";

const GITHUB_URL = "https://github.com/VATUSA/OIS";
const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;

const VERSION = __APP_VERSION__;
const SHA = __APP_SHA__;
// Link the build stamp to its exact commit when we have a SHA, otherwise the repo.
const VERSION_HREF = SHA ? `${GITHUB_URL}/commit/${SHA}` : GITHUB_URL;

const linkClass = "transition-colors hover:text-foreground";

export function Footer() {
  const year = new Date().getFullYear();

  return (
    <footer className="border-t bg-background">
      <div className="mx-auto flex w-full max-w-7xl flex-col gap-4 px-4 py-6 text-sm text-muted-foreground sm:flex-row sm:items-center">
        <div className="flex items-center gap-2">
          <Radar className="size-4 text-primary" />
          <span className="font-medium text-foreground">OIS</span>
          <span className="hidden sm:inline">· VATUSA Operational Information System</span>
        </div>

        <nav className="flex flex-wrap items-center gap-x-4 gap-y-2 sm:ml-auto">
          <Link to="/privacy" className={linkClass}>
            Privacy
          </Link>
          <a href={LICENSE_URL} target="_blank" rel="noreferrer" className={linkClass}>
            License · GPL-3.0
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
            title={SHA ? `Build ${SHA}` : "Source on GitHub"}
          >
            v{VERSION}
            {SHA && ` · ${SHA}`}
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
