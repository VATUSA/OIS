# Engineering prose

Loads for every file. Covers internal engineering text: commit bodies, PR descriptions, review
write-ups, issue comments, design notes in `docs/`, and hand-off reports. Word choice is owned by
`anti-ai-slop.md`; this file is about how the prose is built. Chat replies are `response-style.md`.

Source: ported from the house engineering-prose rule. Comment and commit-body content rules live
in `AGENTS.md` § Conventions & gotchas; commit format and the no-attribution rule live in
`CLAUDE.md` § Standing working agreements. This file does not restate either.

Write like a competent engineer: specific, direct, and willing to commit to a claim. No single
tell proves a text was machine-written. The smell is density, so stop reaching for the patterns.

## Four structural fixes

- **Specific over abstract.** A file:line, a symbol, a number, a failing case. "Dropping
  `.abs()` let a 29-minute-old observation through" beats "improved boundary handling".
- **Active voice, name the actor.** "The reaper pass marks the job failed", not "the job is
  marked failed". Passive voice hides who does what.
- **Lead with the point.** The change or finding first, then the detail.
- **Vary sentence length on purpose.** A long qualified sentence, then a short flat one. Uniform
  15-to-20-word sentences are the core machine texture.

## Lists

- Default to prose. A list is for discrete parallel items: steps, options, failing cases. When the
  points build an argument, the connecting words are the argument.
- Don't open every bullet with a bold keyword. Rule files like this one are the exception.
- Every bullet carries weight. Two real points beat five padded ones.

## PR descriptions

**Write the PR for a junior developer.** Assume they know Rust and React but nothing about this
corner of OIS, this issue, or the decisions behind it. Say what is being fixed in ordinary words
before naming any type or function. A reviewer who has to read the diff to learn what the PR is
for has been failed by the description.

**Draw how it works, before and after, inline.** Put a text diagram in a fenced block in the PR
body. The contrast carries the explanation: the reviewer sees the broken path beside the fixed
one. When the change is about how data moves, draw a data-flow diagram showing where data
enters, what touches it, and where it lands. Most OIS changes are that shape (feed → caches →
handlers, handler → `integration.outbound_jobs` → bot, contract → generated client → web).

The shape (illustrative, not a real change):

```
before:  handler ─▶ channel lookup by old name ─▶ None ─▶ skip enqueue, 200, nothing logged
after:   migration repoints the stored rows to the new name
         handler ─▶ channel lookup ─▶ Some ─▶ outbound_jobs ─▶ bot ─▶ Discord
```

Never attach a file or publish an artifact for the diagram.

A PR body also carries:

- The issue reference in the form `#123 [summary] (Status)` (see `docs/github-issues.md`), plus a
  `Closes #N` line.
- A test plan that names the gates you ran and **the gates you did not run**, with the reason.
  "`just ci` green" and "CI green" are different claims; see `AGENTS.md` § Commands.
- Any deviation from the issue's acceptance criteria, stated plainly. A deviation declared only in
  the PR is still declared, but repeat it on the issue thread so a later reader finds it.

## Commit messages

- Subject: imperative, present tense, `type(scope): summary`, about 70 characters at most. The
  git log is the style reference.
- The body says what changed and why. Name the root cause and the fix and skip the debugging
  play-by-play. Repro detail belongs in the test.
- Don't restate the diff. Explain what the diff cannot show.
- No attribution trailer of any kind; see `CLAUDE.md`.

## Write the mechanism only after you have proved it

A causal claim in a commit body, PR, issue comment, or code comment outlives the session and is
read as settled fact. Verify first, then write. If the reasoning is load-bearing and you have not
run the check, write "evidence points at", or state only what you did. See `code-quality.md`
§ Verify a claim before you write it down.

## American English

`behavior`, `color`, `center`, `license`, `-ize` (`organize`, `summarize`), double quotes. Code
identifiers, UI labels, and quoted text keep their own spelling.

## Revision pass

After drafting anything longer than a commit subject:

1. Read it back. Anywhere it sounds like a release announcement, flatten it.
2. Check rhythm. If every sentence is the same length, vary them.
3. Hunt clusters: "not X but Y", triads, "it's worth noting", bold-led bullets. One may be fine;
   three means a rewrite.
4. Test every hedge and superlative. Delete it or replace it with the fact behind it.
5. Cut 10%.
