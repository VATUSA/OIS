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
});
