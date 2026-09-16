import ReactMarkdown from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

/**
 * Renders external/user-supplied Markdown safely: GFM (autolinked bare URLs, lists, tables), no raw
 * HTML (escaped, not executed — no `rehype-raw`), links open in a new tab. `remark-breaks` renders a
 * single line break as a hard break — the source text (`eventBodyText`) represents VATUSA's `<br>`
 * tags as a lone `\n`, and CommonMark's default soft break collapses that to a space.
 */
export function Markdown({ children, className }: { children: string; className?: string }) {
  return (
    <div className={className}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks]}
        components={{
          // react-markdown passes an extra `node` prop (the hast element) to every custom
          // component (`passNode: true`, hardcoded); it must never reach a real DOM element.
          a: ({ href, children: linkChildren, node: _node, ...rest }) => (
            <a
              {...rest}
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="text-brand-ink underline underline-offset-2 hover:text-ink"
            >
              {linkChildren}
            </a>
          ),
          ul: ({ children: c, node: _node, ...rest }) => (
            <ul {...rest} className="list-disc pl-5 marker:text-ink-3">
              {c}
            </ul>
          ),
          ol: ({ children: c, node: _node, ...rest }) => (
            <ol {...rest} className="list-decimal pl-5 marker:text-ink-3">
              {c}
            </ol>
          ),
          h1: ({ children: c }) => <h3 className="font-semibold text-ink">{c}</h3>,
          h2: ({ children: c }) => <h3 className="font-semibold text-ink">{c}</h3>,
          h3: ({ children: c }) => <h3 className="font-semibold text-ink">{c}</h3>,
          p: ({ children: c }) => <p className="mb-2 last:mb-0">{c}</p>,
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}
