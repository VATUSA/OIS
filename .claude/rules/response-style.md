# Response style

Loads for every file. How to answer in the terminal. Commit bodies, PRs, and review write-ups are
`engineering-prose.md`; word choice is `anti-ai-slop.md`.

Source: ported from the house response-style rule (#742).

Be concise. Default to the shortest complete answer.

Answer first. No preamble, no restating the question, no "Great question."

No recap at the end. When the answer is done, stop.

No hedging filler: "it's worth noting", "it's important to", "keep in mind".

Don't offer next steps or ask follow-up questions unless asked, or unless you are blocked on a
decision that is the user's to make.

Plain prose over bullets and headers unless the content needs structure.

A simple question gets one to three sentences. Go long only when depth is asked for.

Never pad for politeness. Short and blunt beats warm and long.

**Report a problem in four short parts:** the problem (one line), why (the mechanism), how it may
affect someone (what an operator, controller, or pilot would see), and the risk if it ships.

**When you explain how data moves, draw a data-flow diagram inline in the terminal.** Not
optional. A described flow is harder to check than a drawn one, and most OIS questions are flow
questions: the VATSIM feed into the `AppState` caches and on to the trajectory model's three
callers, or a handler enqueueing to `integration.outbound_jobs` for the bot to lease. Never create
or publish a file or artifact for a diagram.

**Name an issue as `#123 [short summary] (Status)`, never a bare number.** The canonical rule,
including where the status comes from, is in `docs/github-issues.md` § Referring to an issue.

**Say which "done" you mean.** "Ready for review", "pushed, PR open", and "merged" are different
states, and a report that blurs them reads as the strongest one. Say which gates ran and which did
not.
