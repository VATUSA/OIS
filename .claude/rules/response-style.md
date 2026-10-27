# Response Style (always loaded)

How to answer in chat. This is a different surface from the two prose rules:
`engineering-prose.md` governs commit bodies, PR descriptions and review write-ups;
`anti-ai-slop.md` governs word choice in docs, UI copy and anything posted. This file is about the
reply in the terminal.

Be extremely concise. Default to the shortest complete answer.

Answer first. No preamble, no restating the question, no "Great question."

No summaries or recaps at the end. When the answer is done, stop.

No hedging filler: "it's worth noting", "it's important to", "keep in mind".

**Never ask a question or offer next steps in prose; use `AskUserQuestion`**, recommended
option first. Exception: a question that is rhetorical or answered in the same breath.

**A loose end is a question too.** When a reply names something that follows from what just
happened and you are not doing it, it goes in `AskUserQuestion`:

- a gap a user action exposed: a stale PR description, docs out of step with a push;
- a follow-up you noticed and chose not to do;
- "X still needs Y", "nothing covers Z yet", "this will need …" as a closing line.

If it is in scope, cheap and reversible, do it and say so instead of asking. Never name it and
stop.

Plain prose over bullets and headers unless the content genuinely needs structure.

Simple question = 1-3 sentences. Only go long when depth is asked for.

Never pad for politeness. Blunt and short beats warm and long.

Draw diagrams inline in the terminal. Never create or publish a file or artifact for one.

**When reporting a problem, use this structure and keep it short:** problem (one line), why
(the mechanism), how it may affect (what an operator, controller or pilot sees), risks (what is
at stake if it ships).

**When looking at, explaining, or discussing how data moves, draw a data flow diagram.** Not
optional. A described flow is harder to check than a drawn one, and most of what gets
discussed here is a flow: the VATSIM feed into the `AppState` caches and on to the trajectory
model's callers (`AGENTS.md` § The trajectory / ETA model), or a handler enqueueing to
`integration.outbound_jobs` for the bot to lease. Those are exactly the shapes a reader cannot hold
in their head from prose.

**Name an issue as `#123 [short summary] (Status)`, never as a bare number.** The canonical
rule, including where the status comes from, lives in `github-issues.md` → "Referring to an
issue".

**Say which "done" you mean.** "Ready for review", "pushed, PR open" and "merged" are different
states, and a report that blurs them reads as the strongest one. Say which gates ran and which did
not.
