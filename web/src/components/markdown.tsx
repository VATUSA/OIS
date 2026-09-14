import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Renders external/user-supplied Markdown safely: GFM (autolinked bare URLs, lists, tables), no raw
 * HTML (escaped, not executed — no `rehype-raw`), links open in a new tab.
 */
export function Markdown({ children, className }: { children: string; className?: string }) {
  return (
    <div className={className}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children: linkChildren }) => (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="underline underline-offset-2 hover:text-foreground"
            >
              {linkChildren}
            </a>
          ),
          ul: ({ children: c }) => <ul className="list-disc pl-5">{c}</ul>,
          ol: ({ children: c }) => <ol className="list-decimal pl-5">{c}</ol>,
          h1: ({ children: c }) => <h3 className="font-semibold">{c}</h3>,
          h2: ({ children: c }) => <h3 className="font-semibold">{c}</h3>,
          h3: ({ children: c }) => <h3 className="font-semibold">{c}</h3>,
          p: ({ children: c }) => <p className="mb-2 last:mb-0">{c}</p>,
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
