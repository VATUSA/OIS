import {Link} from "@tanstack/react-router";
import {BookOpen, Code2} from "lucide-react";

import vatusaLogo from "@/assets/vatusa-logo.png";
import {DOCS_URL} from "@/lib/api";

const GITHUB_URL = "https://github.com/VATUSA/OIS";
const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE.md`;

// Full version like "1.0.1-a1b2c3d". The trailing segment is the build commit when it looks like a
// short SHA — link the stamp straight to that commit, else fall back to the repo.
const VERSION = __APP_VERSION__;
const COMMIT = /^[0-9a-f]{7,40}$/.test(VERSION.split("-").pop() ?? "")
  ? VERSION.split("-").pop()!
  : "";
const VERSION_HREF = COMMIT ? `${GITHUB_URL}/commit/${COMMIT}` : GITHUB_URL;

const linkClass = "inline-flex items-center gap-1.5 transition-colors hover:text-ink";

/** The quiet hairline-topped footer at the bottom of every scrolling page. */
export function Footer() {
  const year = new Date().getFullYear();

  return (
    <footer className="border-t border-line px-4 py-4 text-xs text-ink-3 sm:px-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:gap-4">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <img src={vatusaLogo} alt="" className="size-3.5" />
          <span className="font-semibold text-ink-2">OIS</span>
          <span className="hidden md:inline">· VATUSA Events · Operational Information System</span>
        </div>

        <nav className="flex flex-wrap items-center gap-x-4 gap-y-2 font-semibold sm:ml-auto">
          <Link to="/privacy" className={linkClass}>
            Privacy
          </Link>
          <a href={LICENSE_URL} target="_blank" rel="noreferrer" className={linkClass}>
            License · MIT
          </a>
          <a href={GITHUB_URL} target="_blank" rel="noreferrer" className={linkClass}>
            <Code2 className="size-3.5" />
            GitHub
          </a>
          {DOCS_URL && (
            <a href={DOCS_URL} target="_blank" rel="noreferrer" className={linkClass}>
              <BookOpen className="size-3.5" />
              Docs
            </a>
          )}
          <a
            href={VERSION_HREF}
            target="_blank"
            rel="noreferrer"
            className={"font-mono font-normal " + linkClass}
            title={COMMIT ? `Build commit ${COMMIT}` : "Source on GitHub"}
          >
            v{VERSION}
          </a>
        </nav>
      </div>
      <p className="mt-2">
        © {year} VATUSA. Not affiliated with any government or aviation authority — for use on the
        VATSIM network only.
      </p>
    </footer>
  );
}
