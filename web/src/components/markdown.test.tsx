import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";

import {Markdown} from "./markdown";

const render = (text: string) => renderToStaticMarkup(<Markdown>{text}</Markdown>);

describe("Markdown", () => {
  it("autolinks a bare URL and opens it safely in a new tab", () => {
    const html = render("See https://ois.vzdc.org/planning/events/12539 for details.");
    expect(html).toContain('href="https://ois.vzdc.org/planning/events/12539"');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
  });

  it("renders a markdown link the same way", () => {
    const html = render("[VATUSA](https://vatusa.net)");
    expect(html).toContain('href="https://vatusa.net"');
    expect(html).toContain(">VATUSA</a>");
  });

  it("renders common markdown: bold and a list", () => {
    const html = render("**Important**\n\n- one\n- two");
    expect(html).toContain("<strong>Important</strong>");
    expect(html).toContain("<li>one</li>");
    expect(html).toContain("<li>two</li>");
  });

  it("escapes raw HTML instead of executing it", () => {
    const html = render('<script>alert("xss")</script> and <img src=x onerror="alert(1)">');
    // No live tags — both are entity-escaped into inert text, not real elements.
    expect(html).not.toContain("<script>");
    expect(html).not.toContain("<img ");
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain("&lt;img");
  });

  it("renders a single line break as a hard break, not a collapsed space", () => {
    // eventBodyText() represents VATUSA's <br> tags as a lone \n (only 3+ newlines collapse to
    // \n\n). Without remark-breaks, CommonMark treats that as a soft break — rendered as a plain
    // space by the browser — so an existing <br>-formatted description would silently run its
    // lines together.
    const html = render("Line one\nLine two");
    expect(html).toContain("<br");
  });

  it("passes through a link's title attribute", () => {
    const html = render('[VATUSA](https://vatusa.net "Visit VATUSA")');
    expect(html).toContain('title="Visit VATUSA"');
  });

  it("never leaks react-markdown's internal node prop onto a rendered element", () => {
    // react-markdown hardcodes `passNode: true`, so every custom component receives an extra
    // `node` prop (the hast AST element) alongside real DOM props. Spreading it onto a real
    // element stringifies it as `node="[object Object]"` — a bogus attribute on every link.
    const html = render("[VATUSA](https://vatusa.net)\n\n- one\n- two");
    expect(html).not.toContain("node=");
  });

  it("preserves a numbered list's start value instead of always starting at 1", () => {
    const html = render("3. three\n4. four\n5. five");
    expect(html).toContain('start="3"');
  });
});
