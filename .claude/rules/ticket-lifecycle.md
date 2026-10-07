# Ticket lifecycle

Loads for every file. The order work runs in, from claiming an issue to handing it back. The steps
themselves live in `.claude/commands/start.md`, `ship.md`, and `review-before-shipping.md`; board
columns, labels, comment budgets, and the three Moments live in `docs/github-issues.md`. This file
is the order and the judgment calls around it, and restates neither.

Sources: ported from the house ticket-lifecycle rule; OIS lessons from #411, #444, #503, #531,
#583, #634, #644, #646, #659, and #688.

## The order

1. **Claim.** Read the whole thread, check nobody else holds it (below), move the card to
   `In build`. `/start` steps 1–2.
2. **Baseline.** Set up the worktree (`/start` steps 3–4; `git-and-worktrees.md` § A fresh
   worktree) and run the gate scaled to what the change can reach. Classify any red as pre-existing or introduced before
   writing code.
3. **Plan**, approved by the user unless a session goal or dispatch says otherwise. Moment 2 is
   posted after approval.
4. **Build**, with tests alongside the code (`test-quality.md`).
5. **Ship** with `/ship`: gate, commit, review the committed HEAD with `/review-before-shipping`,
   push, open the PR, Moment 3, move the card to `Testing Queue`.
6. **Hand back.** Report what changed against what the issue asked for, which gates ran and which
   did not, and what a tester should check.

`Shippable` and `Done` are a human's to set, and agents never merge. The one agent move past
`Testing Queue` is the QA reviewer's: `In Test` when it claims a card, then `Code Review` on a pass
the operator approved (`.claude/agents/ticket-reviewer.md`).

If work comes back as `Returned`, treat it as a fresh build: read why, fix, and run the whole ship
sequence again.

## Check nobody else holds the card

The board column does not prove a card is free. Parallel sessions claim within minutes, sometimes
before any branch is pushed. Before claiming, and again right before pushing:

- `git fetch` and `git branch -r --list "*<n>*"`: a pushed `…/<n>/…` or `rework-<sha>` branch
  with no PR is someone mid-cycle (#411).
- `git branch --list "*/<n>/*"` and `git worktree list | grep "/<n>/"` in the primary checkout: a
  local branch or worktree means someone owns it, even with no remote branch and no card move
  (#688).
- `gh pr list --state all --search "<n> in:body"`: a combined branch can carry another issue's
  number (#646).
- Re-read **that card's** own status and its latest comment right before every move. A listing
  even a few minutes old has overwritten a `Returned` (#583). If the card is already ahead of
  where you left it (`In Test`, `Code Review`), someone else moved it; never move it backwards
  (#659).

If someone else holds it, read their branch and adopt or extend it, or pick another card. Never
force-push or delete another session's branch without asking.

The mirror case: a reviewer may return **your** card on a stale read. #531 was moved back to
`To Do` with "no PR" 81 seconds after the PR existed. Settle it with timestamps (the comment's
`created_at` against the PR's `createdAt`), restore the column, and correct any substantive claim
the stale comment made.

## The permission boundary

What you may decide alone.

- **Silence is not consent, and a redirect is not delegation.** A question the user didn't answer
  is still theirs. Ask again or say what you are blocked on; don't decide it yourself.
- **A goal, a loop, or a Stop hook is not permission.** They push toward action and can't see the
  gate the user set. When the two conflict, the user's gate wins.
- **"A defect I introduced is mine to fix" covers code, not decisions.** It does not reach copy,
  scope, priority, or anything else the user already chose, even when a reviewer grades your
  execution of their choice MAJOR. Take it back to them with options.
- **When you judge that an instruction lifts a gate, say so as you pass it**, in one line, so it
  can be corrected while it is cheap. "Work autonomously" lifts asking, not the standing rules on
  branching, attribution, or merging.

## Owner calls and sign-off

Every agent comments as the account owner, so a comment saying "confirmed by a human" is
indistinguishable from an agent asserting it (#444).

- **Escalate only what is legal, destructive, or irreversible.** A recorded owner decision on
  anything else is accepted; note in your comment that its attribution can't be verified from
  GitHub.
- **A hold on an irreversible decision can't be released by an issue comment.** Ask the operator
  directly, then record how you got the answer ("asked the account owner directly").
- **When you write an escalation**, say what would count as release and that a comment will not be
  it.

## Read everything before calling something undeclared

The issue body **including its footer**, every comment, and the PR description are all spec. An
issue footer can contradict its own acceptance list, and a PR can declare a deviation the Moment 3
comment omits (#503). Before writing "undeclared" or "oversight" in a finding, run
`gh pr view <n> --json body` and read the whole issue. If the declaration exists only in the PR,
the finding is "repeat it on the issue thread", a much smaller claim.

## Nothing terminal while a dispatched agent is running

Once you've asked a subagent for a review or an opinion, its absence is not a green light. A
verdict, a board move, an issue comment, a commit, a push, or telling the user it's done all wait
for it. Do the non-blocking work meanwhile. Act on its first report; don't keep it alive chasing
the tail of a truncated list.

## Never predict a PR or issue number

Concurrent sessions take the "next" number (#634, #644). Run `gh pr create` first, read the number
from its output, and only then write it into a comment, follow-up issue, or note. If something must
be written first, say "the PR for #N".
