#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/attribution-gate.sh, its parser lib/attribution_targets.py, the shared
# patterns in .claude/hooks/lib/attribution.sh, and the git hook .githooks/commit-msg that uses them.
#
#   bash bin/lib/attribution-gate.test.sh
#
# OIS credits nothing to an agent (AGENTS.md -> Git workflow, #754): every attribution form, through
# every command that posts a commit, PR, issue or comment body, must block, while prose that only names
# a form (in a code span, or with words around it) and ordinary bodies pass. Each case runs the gate
# with a payload whose cwd is a scratch directory holding the body files. Exits non-zero on any failure.
# ==============================================================================
set -uo pipefail
# shellcheck disable=SC2016  # payload commands are single-quoted on purpose: they are test input

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$ROOT/.claude/hooks/attribution-gate.sh"
COMMIT_MSG="$ROOT/.githooks/commit-msg"
WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/attribution-gate-test.XXXXXX")" && pwd -P)"
trap 'chmod -R u+rw "$WORK" 2>/dev/null; rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

WT="$WORK/wt"
mkdir -p "$WT"

# gate WANT NAME COMMAND — WANT is allow (exit 0) or block (exit 2); the payload's cwd is $WT
gate() {
    local want="$1" rc got
    python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": sys.argv[1]}, "cwd": sys.argv[2]}))' \
        "$3" "$WT" | (cd "$WORK" && bash "$GATE" >"$WORK/out" 2>&1)
    rc=$?
    case "$rc" in 0) got=allow ;; 2) got=block ;; *) got="rc=$rc" ;; esac
    if [[ "$got" == "$want" ]]; then ok "$want: $2"; else bad "$2 (want $want, got $got: $(head -c 300 "$WORK/out"))"; fi
}

echo "Commits, PRs, issues and comments:"
printf 'fix: x\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\n' >"$WT/dirty-msg.txt"
printf 'fix: x\n\nCloses #1\n' >"$WT/clean-msg.txt"
printf '## Summary\nx\n\nhttps://claude.ai/code/session_01AbCdEf\n' >"$WT/dirty-body.md"
printf '## Summary\nx\n' >"$WT/clean-body.md"
gate block "commit -m with a Claude trailer" 'git commit -m "fix: x

Co-Authored-By: Claude <noreply@anthropic.com>"'
gate block "lower-case trailer" 'git commit -m "fix: x" -m "co-authored-by: claude"'
gate block "heredoc commit with a trailer" "$(printf 'git commit -F - <<%sEOF%s\nfix: x\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nEOF' "'" "'")"
gate block "commit -F file with a trailer" 'git commit -F dirty-msg.txt'
gate block "commit --file=file with a trailer" 'git commit --file=dirty-msg.txt'
gate block "pr create --body with Generated with" 'gh pr create --title t --body "🤖 Generated with [Claude Code](https://claude.com/claude-code)"'
gate block "pr create --body-file with a session link" 'gh pr create --title t --body-file dirty-body.md'
gate block "pr edit -F with a session link" 'gh pr edit 5 -F dirty-body.md'
gate allow "clean commit -m" 'git commit -m "fix(flow): clamp descent ETA" -m "Closes #1"'
gate allow "clean commit -F" 'git commit -F clean-msg.txt'
gate allow "message file written by the same command" 'printf "fix: x\n" > new-msg.txt && git commit -F new-msg.txt'
gate allow "clean pr create" 'gh pr create --title t --body-file clean-body.md'
gate allow "trailer text outside a commit or PR" 'echo "Co-Authored-By: Claude" | wc -l'
gate allow "pr view" 'gh pr view 5'
# Bodies that reach gh/git without a plain path (#744 review).
printf 'fix: x\n\nCo-Authored-By: Someone <noreply@anthropic.com>\n' >"$WT/noreply-msg.txt"
gate block "noreply@anthropic.com alone" 'git commit -F noreply-msg.txt'
gate block "pr body from \$(cat file)" 'gh pr create --title t --body "$(cat dirty-body.md)"'
gate block "pr body from \$(< file)" 'gh pr edit 5 --body "$(< dirty-body.md)"'
gate block "commit message from \$(cat file)" 'git commit -m "$(cat dirty-msg.txt)"'
gate block "pr body piped to --body-file -" 'cat dirty-body.md | gh pr create --title t --body-file -'
gate block "pr body from /dev/stdin" 'gh pr create --title t --body-file /dev/stdin < clean-body.md'
gate block "commit message piped to -F -" 'cat clean-msg.txt | git commit -F -'
gate block "glued -Ffile" 'gh pr create --title t -Fdirty-body.md'
gate block "\$(cat file) that is missing" 'gh pr create --title t --body "$(cat no-such-body.md)"'
gate allow "pr body from \$(cat clean file)" 'gh pr create --title t --body "$(cat clean-body.md)"'
gate allow "commit -m from a heredoc substitution" "$(printf 'git commit -m "$(cat <<%sEOF%s\nfix: x\n\nCloses #1\nEOF\n)"' "'" "'")"
gate allow "body file written by the same heredoc command" "$(printf 'cat > new-body.md <<%sEOF%s\nclean\nEOF\ngh pr create --title t --body "$(cat new-body.md)"' "'" "'")"
# Issues, comments and the Drafted-by footer (#787). Every attribution form, through every command
# that posts an issue or comment body, blocks.
printf 'x\n\nCo-Authored-By: Claude\n' >"$WT/form-coauthor.md"
printf 'x\n\nReviewed-by: Bot <noreply@anthropic.com>\n' >"$WT/form-noreply.md"
printf 'x\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)\n' >"$WT/form-generated.md"
printf 'x\n\nhttps://claude.ai/code/session_01AbCdEf\n' >"$WT/form-session.md"
printf 'x\n\n🤖 Drafted by Claude Code\n' >"$WT/form-drafted.md"
for form in coauthor noreply generated session drafted; do
    b="form-$form.md"
    gate block "issue create, $form" "gh issue create --repo VATUSA/OIS --title t --body-file $b"
    gate block "issue comment, $form" "gh issue comment 5 --body-file $b"
    gate block "issue edit, $form" "gh issue edit 5 -F $b"
    gate block "pr comment, $form" "gh pr comment 5 --body-file=$b"
    gate block "gh api comment PATCH, $form" "gh api -X PATCH repos/VATUSA/OIS/issues/comments/123 -F body=@$b"
    gate block "gh api comment POST, $form" "gh api repos/VATUSA/OIS/issues/5/comments -F body=@$b"
done
# The footer as the forms an agent actually writes it.
drafted_heredoc="$(printf 'gh issue comment 5 --body-file - <<%sEOF%s\nVerified.\n\n🤖 Drafted by Claude Code\nEOF' "'" "'")"
gate block "issue comment heredoc with the footer" "$drafted_heredoc"
gate block "issue create --body with the footer" 'gh issue create --title t --body "Steps.

🤖 Drafted by Claude Code"'
gate block "issue comment -b with a linked footer" 'gh issue comment 5 -b "Done.

_🤖 Drafted by [Claude Code](https://claude.com/claude-code)_"'
gate block "pr review --comment with the footer" 'gh pr review 5 --comment -b "Looks fine.

🤖 Drafted by Claude Code"'
gate block "issue close --comment with the footer" 'gh issue close 5 --comment "Fixed in #9.

🤖 Drafted by Claude Code"'
gate block "issue reopen --comment with a trailer" 'gh issue reopen 5 --comment "Regressed.

Co-Authored-By: Claude"'
gate block "pr close --comment with a session link" 'gh pr close 5 -c "Superseded. https://claude.ai/code/session_01AbCdEf"'
gate block "pr reopen --comment with the footer" 'gh pr reopen 5 --comment "Back.

<sub>🤖 Drafted by Claude Code</sub>"'
gate block "pr create body with the footer" 'gh pr create --title t --body-file form-drafted.md'
gate block "commit heredoc with the footer" "$(printf 'git commit -F - <<%sEOF%s\nfix: x\n\nDrafted by Claude Code.\nEOF' "'" "'")"
gate block "printf body escaped onto one line" "printf 'Done.\\\\n\\\\n🤖 Drafted by Claude Code\\\\n' > new-c.md && gh issue comment 5 --body-file new-c.md"
gate block "gh api -f body= with the footer" "gh api --method PATCH /repos/VATUSA/OIS/issues/comments/9 -f body='Edited.

🤖 Drafted by Claude Code'"
gate block "gh api create issue -f body= with a trailer" 'gh api repos/VATUSA/OIS/issues -f title=t -f "body=x Co-Authored-By: Claude"'
gate block "gh api -XPATCH issue body from \$(cat file)" 'gh api -XPATCH repos/VATUSA/OIS/issues/5 -f body="$(cat form-session.md)"'
gate block "gh api --field=body=@file" 'gh api -X POST repos/VATUSA/OIS/issues/5/comments --field=body=@form-generated.md'
gate block "gh api glued -Fbody=@file" 'gh api repos/VATUSA/OIS/issues/5/comments -Fbody=@form-noreply.md'
printf '{"body": "Verified.\\n\\n🤖 Drafted by Claude Code"}\n' >"$WT/drafted.json"
gate block "gh api --input JSON with an escaped footer" 'gh api -X PATCH repos/VATUSA/OIS/issues/comments/9 --input drafted.json'
gate block "gh api --input= JSON with an escaped footer" 'gh api repos/VATUSA/OIS/issues/5/comments --input=drafted.json'
gate block "gh api --input heredoc with an escaped footer" "$(printf 'gh api repos/VATUSA/OIS/issues/5/comments --input - <<%sEOF%s\n{"body": "ok\\\\n\\\\n🤖 Drafted by Claude Code"}\nEOF' "'" "'")"
gate block "gh api graphql addComment with the footer" "gh api graphql -f query='mutation(\$b: String!) { addComment(input: {subjectId: \"x\", body: \$b}) { clientMutationId } }' -f b='ok

🤖 Drafted by Claude Code'"
gate block "gh api write with a method it cannot read" 'gh api -X "$METHOD" repos/VATUSA/OIS/issues/comments/9 -F body=@form-drafted.md'
# A body that is ONLY the attribution line, with its field or option name attached (`body=`, `--body=`): the
# name used to hide that the line starts there.
for form in '🤖 Drafted by Claude Code' 'Drafted by Claude Code' '<sub>🤖 Drafted by Claude Code</sub>' \
    '🤖 Generated with [Claude Code](https://claude.com/claude-code)' 'Co-Authored-By: Claude <noreply@anthropic.com>'; do
    gate block "gh api POST -f body=, body only: $form" "gh api repos/VATUSA/OIS/issues/5/comments -f body='$form'"
    gate block "gh api PATCH -f \"body=\", body only: $form" "gh api -X PATCH repos/VATUSA/OIS/issues/comments/9 -f \"body=$form\""
    gate block "gh api --raw-field=body=, body only: $form" "gh api repos/VATUSA/OIS/issues/5/comments --raw-field='body=$form'"
    gate block "gh api -F body=, body only: $form" "gh api -X PATCH repos/VATUSA/OIS/issues/comments/9 -F body='$form'"
    gate block "issue comment --body=, body only: $form" "gh issue comment 5 --body='$form'"
    gate block "issue comment --body, body only: $form" "gh issue comment 5 --body '$form'"
    gate block "pr comment -b, body only: $form" "gh pr comment 5 -b '$form'"
    gate block "issue comment --edit-last --body=, body only: $form" "gh issue comment 5 --edit-last --body='$form'"
done
gate allow "control: gh api body= of prose naming the footer" "gh api repos/VATUSA/OIS/issues/5/comments -f body='remove the Drafted by Claude Code footer'"
gate allow "control: issue comment --body= of prose" "gh issue comment 5 --body='Verified on the stack.'"
gate allow "control: issue comment --edit-last with a clean file" "gh issue comment 5 --edit-last --body-file clean-body.md"
gate block "issue comment --edit-last with a footer file" "gh issue comment 5 --edit-last --body-file form-drafted.md"
# A body it cannot read fails closed, as it does for commits and PRs.
mkdir -p "$WT/a-dir.md"
gate block "issue comment body piped to --body-file -" 'cat clean-body.md | gh issue comment 5 --body-file -'
gate block "issue create body from /dev/stdin" 'gh issue create --title t -F /dev/stdin < clean-body.md'
gate block "gh api comment body piped to --input -" 'cat drafted.json | gh api -X PATCH repos/VATUSA/OIS/issues/comments/9 --input -'
gate block "gh api comment body from -F body=@-" 'cat clean-body.md | gh api repos/VATUSA/OIS/issues/5/comments -F body=@-'
gate block "issue comment body from a missing \$(cat file)" 'gh issue comment 5 --body "$(cat no-such-comment.md)"'
gate block "issue comment body file that is a directory" 'gh issue comment 5 --body-file a-dir.md'
gate block "gh api --input that is a directory" 'gh api -X PATCH repos/VATUSA/OIS/issues/comments/9 --input a-dir.md'
printf 'secret\n' >"$WT/unreadable.md" && chmod 000 "$WT/unreadable.md"
if [[ ! -r "$WT/unreadable.md" ]]; then
    gate block "issue comment body file it cannot read" 'gh issue comment 5 --body-file unreadable.md'
fi
# Prose that only describes the rule, and ordinary bodies, pass.
printf '## Decision\nRemove the `🤖 Drafted by Claude Code` requirement from every agent comment.\nNo `Drafted by` or `Generated with` line, ever.\nThe Drafted by Claude Code footer was posted on #756 and edited by hand.\nDrafted by Claude Code footers are banned on every comment.\nNo comment may be signed Drafted by Claude Code.\n' >"$WT/rule-prose.md"
printf 'Verified on the stack.\n\n- step one\n- step two\n' >"$WT/plain-comment.md"
printf '{"body": "Remove the `🤖 Drafted by Claude Code` requirement.\\nDone."}\n' >"$WT/prose.json"
for b in rule-prose.md plain-comment.md; do
    gate allow "issue create, $b" "gh issue create --title t --body-file $b"
    gate allow "issue comment, $b" "gh issue comment 5 --body-file $b"
    gate allow "issue edit, $b" "gh issue edit 5 --body-file $b"
    gate allow "pr comment, $b" "gh pr comment 5 -F $b"
    gate allow "gh api comment PATCH, $b" "gh api -X PATCH repos/VATUSA/OIS/issues/comments/123 -F body=@$b"
    gate allow "gh api comment POST, $b" "gh api repos/VATUSA/OIS/issues/5/comments -F body=@$b"
done
gate allow "decision comment quoting the footer inline" 'gh issue comment 754 --body "Owner decision: remove the `🤖 Drafted by Claude Code` requirement."'
gate allow "gh api --input JSON of prose" 'gh api -X PATCH repos/VATUSA/OIS/issues/comments/9 --input prose.json'
gate allow "the documented pre-post check" "! grep -qiE 'Drafted by|Generated with|Co-Authored' rule-prose.md && gh issue comment 5 --body-file rule-prose.md"
gate allow "comment heredoc without a footer" "$(printf 'gh issue comment 5 --body-file - <<%sEOF%s\nVerified.\nEOF' "'" "'")"
gate allow "gh api GET filtering comments for a trailer" "gh api repos/VATUSA/OIS/issues/5/comments --jq '.[] | select(.body | test(\"Co-Authored-By: Claude\"))'"
gate allow "gh api -X GET search with fields" 'gh api -X GET search/issues -f q="Co-Authored-By: Claude in:comments"'
gate allow "gh issue view" 'gh issue view 5 --comments'
gate allow "gh issue list searching for the footer" 'gh issue list --search "Co-Authored-By: Claude"'
# Review of #787: an inline body followed by more arguments, JSON bodies, quoted field paths,
# files made by a command the gate cannot read, gh aliases, and prose that quotes a form.
gate block "inline footer, then 2>&1" 'gh issue comment 5 --body "Verified.\n\n🤖 Drafted by Claude Code" 2>&1'
gate block "inline footer, then --repo" 'gh issue comment 5 --body "Verified.

🤖 Drafted by Claude Code" --repo VATUSA/OIS'
gate block "inline footer, then && echo" 'gh issue comment 5 --body "Verified.

🤖 Drafted by Claude Code" && echo posted'
gate block "inline footer, then --title" 'gh issue create --body "Steps.

🤖 Drafted by Claude Code" --title t'
gate block "gh api -f body= footer, then the endpoint" "gh api -X PATCH -f body='Edited.

🤖 Drafted by Claude Code' repos/VATUSA/OIS/issues/comments/9"
gate block "ANSI-C body with an escaped emoji, then --repo" "gh issue comment 5 --body \$'Done.\\n\\n\\U0001F916 Drafted by Claude Code' --repo VATUSA/OIS"
gate block "footer inside bash -c, then --repo" "bash -c 'gh issue comment 5 --body \"ok\\n\\n🤖 Drafted by Claude Code\" --repo x'"
gate allow "inline prose quoting the footer in double quotes" 'gh issue comment 5 --body "The \"Drafted by Claude Code\" footer is banned." --repo VATUSA/OIS'
gate allow "inline prose quoting the footer in single quotes" "gh issue comment 5 --body 'No \"Drafted by Claude Code\" line.' --repo VATUSA/OIS"
# JSON bodies as jq -c and Python's json.dumps (ensure_ascii) write them, and GraphQL inline.
printf '{"body":"Done.\\n\\n🤖 Drafted by Claude Code","title":"t"}\n' >"$WT/not-last.json"
printf '{"body": "Done.\\n\\n\\ud83e\\udd16 Drafted by Claude Code", "title": "t"}\n' >"$WT/ascii.json"
printf '{"body": "Done.\\r\\n\\r\\n🤖 Drafted by Claude Code\\r\\n"}\n' >"$WT/crlf.json"
printf '{"body":"Remove the `🤖 Drafted by Claude Code` requirement.","title":"t"}\n' >"$WT/prose-not-last.json"
gate block "--input JSON, body not the last key" 'gh api repos/VATUSA/OIS/issues/5/comments --input not-last.json'
gate block "--input JSON, escaped emoji" 'gh api repos/VATUSA/OIS/issues/5/comments --input ascii.json'
gate block "-XPOST --input= JSON, escaped emoji" 'gh api -XPOST repos/VATUSA/OIS/issues/5/comments --input=ascii.json'
gate block "--input JSON, CRLF body" 'gh api -X PATCH repos/VATUSA/OIS/issues/comments/9 --input crlf.json'
gate block "-F body=@ JSON file, escaped emoji" 'gh api repos/VATUSA/OIS/issues/5/comments -F body=@ascii.json'
gate block "graphql mutation, body inline and not last" "gh api graphql -f query='mutation { addComment(input: {body: \"ok\\n\\n🤖 Drafted by Claude Code\", subjectId: \"I_x\"}) { clientMutationId } }'"
gate allow "--input JSON of prose, body not the last key" 'gh api repos/VATUSA/OIS/issues/5/comments --input prose-not-last.json'
# A field path the parser cannot see behind quotes fails closed; typed fields with spaces do not.
gate block "gh api -F \"body=@\$F\"" 'F=form-drafted.md; gh api repos/VATUSA/OIS/issues/5/comments -F "body=@$F"'
gate block "gh api -F \"body=@\$TMPDIR/x\"" 'gh api repos/VATUSA/OIS/issues/5/comments -F "body=@$TMPDIR/x.md"'
gate block "gh api --field=\"body=@\$F\"" 'gh api repos/VATUSA/OIS/issues/5/comments --field="body=@$F"'
gate block "gh api -F 'body=@my dir/x'" "gh api repos/VATUSA/OIS/issues/5/comments -F 'body=@my dir/x.md'"
gate allow "gh api -F \"body=@plain.md\" and a spaced typed field" 'gh api repos/VATUSA/OIS/issues -f title=t -F "labels[]=a b" -F "body=@plain-comment.md"'
# Files made in the same line by something other than a heredoc, and other substitutions.
gate block "echo appends the footer to an existing clean file" 'echo "🤖 Drafted by Claude Code" >> clean-body.md && gh issue comment 5 --body-file clean-body.md'
gate block "echo -e writes the footer" "echo -e 'Done.\\n\\n🤖 Drafted by Claude Code' > new-e.md && gh issue comment 5 --body-file new-e.md"
gate block "printf %s writes the footer" "printf '%s\\n\\n%s\\n' 'Done.' '🤖 Drafted by Claude Code' > new-p.md && gh issue comment 5 --body-file new-p.md"
gate block "cp makes the body file" 'cp form-drafted.md new-cp.md && gh issue comment 5 --body-file new-cp.md'
gate block "gh api --input made by cp" 'cp drafted.json new-cp.json && gh api repos/VATUSA/OIS/issues/5/comments --input new-cp.json'
gate block "body from \$(head file)" 'gh issue comment 5 --body "$(head -50 form-drafted.md)"'
gate block "body from \$(cat file | sed)" 'gh issue comment 5 --body "$(cat form-drafted.md | sed s/x/y/)"'
gate block "gh api -f body= from \$(jq)" 'gh api repos/VATUSA/OIS/issues/5/comments -f body="$(jq -r .body drafted.json)"'
gate block "commit -m from \$(git log)" 'git commit -m "$(git log -1 --format=%B)"'
gate block "printf writes the footer with CRLF escapes" "printf 'Done.\\r\\n\\r\\n🤖 Drafted by Claude Code\\r\\n' > new-r.md && gh issue comment 5 --body-file new-r.md"
gate block "printf writes the footer with a \\x-escaped emoji" "printf 'Done.\\n\\n\\xf0\\x9f\\xa4\\x96 Drafted by Claude Code\\n' > new-x.md && gh issue comment 5 --body-file new-x.md"
gate allow "body file written by tee" "printf 'ok\\n' | tee new-t.md >/dev/null && gh issue comment 5 --body-file new-t.md"
gate allow "--input written by a redirect" "jq -n '{body: \"ok\"}' > new-j.json && gh api repos/VATUSA/OIS/issues/5/comments --input new-j.json"
gate allow "a title from \$(git log)" 'gh pr create --title "$(git log -1 --format=%s)" --body-file clean-body.md'
# gh's own aliases, and the squash commit pr merge writes.
gate block "issue new" 'gh issue new --title t --body-file form-drafted.md'
gate block "pr new" 'gh pr new --title t --body-file form-coauthor.md'
gate block "pr merge --body with a trailer" 'gh pr merge 5 --squash --body "fix: x

Co-Authored-By: Claude"'
gate block "pr merge --body-file with a trailer" 'gh pr merge 5 --squash --body-file form-coauthor.md'
gate block "pr merge -F with the footer" 'gh pr merge 5 --squash -F form-drafted.md'
gate allow "pr merge with a clean body" 'gh pr merge 5 --squash --body-file clean-body.md'
# Prose that names every form in a code span, as a QA comment on this issue would, passes; a footer
# that is only a code span is a quotation. A blockquote of the footer is still the footer.
printf 'Verified: the hook now blocks `Co-Authored-By: Claude`, `noreply@anthropic.com`,\n`🤖 Generated with [Claude Code](https://claude.com/claude-code)`, `claude.ai/code/session_x` and\n`🤖 Drafted by Claude Code` on issue comments.\n\n`🤖 Drafted by Claude Code`\n' >"$WT/qa-prose.md"
printf 'What was posted on #756:\n\n> 🤖 Drafted by Claude Code\n' >"$WT/blockquote.md"
gate allow "issue comment naming every form in code spans" 'gh issue comment 787 --body-file qa-prose.md'
gate allow "gh api comment naming every form in code spans" 'gh api repos/VATUSA/OIS/issues/787/comments -F body=@qa-prose.md'
gate allow "inline prose naming a trailer in a code span" "gh issue comment 787 --body 'Blocks \`Co-Authored-By: Claude\` now.' --repo VATUSA/OIS"
gate block "a blockquoted footer" 'gh issue comment 5 --body-file blockquote.md'
printf 'x\n\n🤖 Drafted by Claude Code (Opus 5.5)\n' >"$WT/drafted-note.md"
printf 'x\n\n🤖 Drafted by Claude Code https://claude.com/claude-code\n' >"$WT/drafted-url.md"
printf 'x\n\n🤖 Drafted with Claude Code\n' >"$WT/drafted-with.md"
for b in drafted-note.md drafted-url.md drafted-with.md; do
    gate block "footer variant $b" "gh issue comment 5 --body-file $b"
done
# Method and field parsing: reads pass with a trailer in their fields, and each write form blocks.
for m in '-XGET' '--method=GET' '--method GET' '-X HEAD' '-X=get'; do
    gate allow "gh api $m with fields naming a trailer" "gh api $m search/issues -f q=\"Co-Authored-By: Claude in:comments\""
done
gate block "gh api glued -fbody= with no method" 'gh api repos/VATUSA/OIS/issues/5/comments -fbody="Co-Authored-By: Claude"'
gate block "gh api --raw-field=body= with no method" 'gh api repos/VATUSA/OIS/issues/5/comments --raw-field=body="Co-Authored-By: Claude"'
gate block "gh api -X=PATCH" 'gh api -X=PATCH repos/VATUSA/OIS/issues/comments/9 -F body=@form-drafted.md'
gate block "gh -R X api -F body=@file" 'gh -R VATUSA/OIS api repos/VATUSA/OIS/issues/5/comments -F body=@form-drafted.md'

echo "Parsed, not grepped:"
gate allow "a commit message naming the gate"           'git log --grep "Co-Authored-By: Claude" --oneline'
gate allow "a heredoc written to a file naming a commit" "$(printf 'cat > notes.md <<%sEOF%s\ngit commit -m "Co-Authored-By: Claude"\nEOF' "'" "'")"
gate block "a commit inside bash -c"                    "bash -c 'git commit -m \"fix: x

Co-Authored-By: Claude\"'"
gate block "a commit behind cd"                         'cd . && git commit -F dirty-msg.txt'
gate block "an absolute message file"                   "git commit -F $WT/dirty-msg.txt"
gate block "a commit with -C before the subcommand"     "git -C $WT commit -F $WT/dirty-msg.txt"
gate block "a relative file on a line that moves"       'cd sub; git commit -F clean-msg.txt'
gate allow "cd ABSDIR && commit with a relative file"   "cd $WT && git commit -F clean-msg.txt"
gate block "gh -R before the group"                     'gh -R VATUSA/OIS issue comment 5 --body-file form-drafted.md'
gate block "an unparseable line naming a commit"        'git commit -m "unterminated'
gate block "ANSI-C quoting on a commit line"            "git commit -m \$'fix\\n\\nCo-Authored-By: Claude'"
gate allow "an unrelated command"                       'cargo test --workspace'
gate allow "an unrelated command mentioning gh"          'ls ~/.config/gh'

echo "Fails closed on a payload it cannot read:"
for payload in "" "this is not json"; do
    printf '%s' "$payload" | bash "$GATE" >/dev/null 2>&1
    if [[ $? -eq 2 ]]; then ok "block: payload '${payload:-<empty>}'"; else bad "payload '${payload:-<empty>}' was not blocked"; fi
done
python3 -c 'import json; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": "git commit -m x"}}))' \
    | (cd "$WT" && PATH=/nonexistent /bin/bash "$GATE" >/dev/null 2>&1)
[[ $? -eq 2 ]] && ok "block when python3 is missing" || bad "python3 missing was not blocked"
mkdir -p "$WORK/planted" && printf 'raise SystemExit(0)\n' > "$WORK/planted/json.py"
python3 -c 'import json,sys; print(json.dumps({"tool_name": "Bash", "tool_input": {"command": "git commit -F dirty-msg.txt"}, "cwd": sys.argv[1]}))' "$WT" \
    | (cd "$WORK/planted" && bash "$GATE" >/dev/null 2>&1)
[[ $? -eq 2 ]] && ok "a planted json.py in the cwd does not disarm it" || bad "planted json.py disarmed the gate"

echo "The git hook .githooks/commit-msg shares the patterns:"
# hook_commit WANT NAME MESSAGE — WANT is reject (non-zero) or accept (0)
hook_commit() {
    local want="$1" got
    printf '%s\n' "$3" > "$WORK/COMMIT_EDITMSG"
    bash "$COMMIT_MSG" "$WORK/COMMIT_EDITMSG" >/dev/null 2>&1
    got=$?
    if { [[ "$want" == reject ]] && [[ $got -ne 0 ]]; } || { [[ "$want" == accept ]] && [[ $got -eq 0 ]]; }; then
        ok "commit-msg $want: $2"
    else
        bad "commit-msg: $2 (want $want, exit $got)"
    fi
}
hook_commit reject "Co-Authored-By: Claude trailer" $'fix: x\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
hook_commit reject "Generated with Claude Code line" $'fix: x\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)'
hook_commit reject "Drafted by Claude Code footer" $'fix: x\n\n🤖 Drafted by Claude Code'
hook_commit accept "prose that names the footer" $'docs: drop the `🤖 Drafted by Claude Code` requirement\n\nNo Drafted by Claude Code footer on comments either.'
hook_commit accept "clean message" $'fix: x\n\nCloses #1'
hook_commit reject "Drafted-with footer with a note" $'fix: x\n\n🤖 Drafted with Claude Code (Opus 5.5)'
hook_commit accept "prose that names the trailer in a code span" $'docs: x\n\nNo `Co-Authored-By: Claude` trailer, no `noreply@anthropic.com`.'

echo ""
echo "attribution-gate: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
