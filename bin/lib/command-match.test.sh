#!/bin/bash
# ==============================================================================
# Tests for .claude/hooks/lib/command_match.py.
#
#   bash bin/lib/command-match.test.sh
#
# A gate must fire on every way a shell could run the command, and stay quiet when
# the words are only data (a heredoc written to a file, a quoted argument).
# Self-reports pass/fail, exits non-zero on any failure.
# ==============================================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MATCH="$ROOT/.claude/hooks/lib/command_match.py"

PASS=0
FAIL=0
ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# expect WANT(0 invoked | 1 not | 2 unparseable) NAME COMMAND
expect() {
    local got
    printf '%s' "$3" | python3 "$MATCH" gh pr create >/dev/null 2>&1
    got=$?
    if [[ "$got" == "$1" ]]; then ok "$2"; else bad "$2 (want $1, got $got)"; fi
}

# gated NAME COMMAND: the gate fires, by a match (0) or by failing closed (2); never "not invoked" (1).
gated() {
    local got
    printf '%s' "$2" | python3 "$MATCH" gh pr create >/dev/null 2>&1
    got=$?
    if [[ "$got" == 0 || "$got" == 2 ]]; then ok "$1"; else bad "$1 (want 0 or 2, got $got)"; fi
}

T='gh pr create'
NL=$'\n'

echo "Nesting is bounded:"
expect 0 "eight nested evals still match"         "eval eval eval eval eval eval eval eval $T"
expect 2 "ten nested evals fail closed"           "eval eval eval eval eval eval eval eval eval eval $T"

echo "Invoked:"
expect 0 "bare"                               "$T --base next --title x"
expect 0 "after cd &&"                        "cd /repo && $T --title x"
expect 0 "after ;"                            "true; $T"
expect 0 "after ||"                           "false || $T"
expect 0 "on its own line"                    "echo hi${NL}$T"
expect 0 "piped"                              "$T | tee out"
expect 0 "backgrounded"                       "$T &"
expect 0 "in a subshell"                      "($T)"
expect 0 "env assignment prefix"              "GH_TOKEN=x $T"
expect 0 "command wrapper"                    "command $T"
expect 0 "env wrapper"                        "env A=b $T"
expect 0 "absolute path"                      "/usr/bin/gh pr create"
expect 0 "extra spaces"                       "gh  pr   create"
expect 0 "bash -c"                            "bash -c \"$T --title x\""
expect 0 "eval"                               "eval \"$T\""
expect 0 "command substitution"               "url=\$($T --title x)"
expect 0 "heredoc body passed via \$(cat)"     "$T --title x --body \"\$(cat <<'EOF'${NL}## Summary${NL}EOF${NL})\""
expect 1 "commit form with parens, backticks and \$ in the body" "git commit -m \"\$(cat <<'EOF'${NL}Fix (x) \`y\` \$(z) it's${NL}${NL}Co-Authored-By: A <a@b>${NL}EOF${NL})\""
expect 0 "commit form then the command"          "git commit -m \"\$(cat <<'EOF'${NL}msg${NL}EOF${NL})\" && $T"
gated "two quoted blocks then the command"      "a=\"\$(cat <<'EOF'${NL}x${NL}EOF${NL})\" b=\"\$(cat <<'EOF'${NL}y${NL}EOF${NL})\"; $T"
expect 0 "backticks"                          "url=\`$T\`"
expect 0 "heredoc fed to bash"                "bash <<'EOF'${NL}$T${NL}EOF"
expect 0 "heredoc fed to sh -s"               "sh -s <<EOF${NL}cd x && $T${NL}EOF"
expect 0 "command after a data heredoc"       "cat > f <<'EOF'${NL}x${NL}EOF${NL}$T"

echo "git global options before the subcommand:"
expect_git() {
    local got
    printf '%s' "$3" | python3 "$MATCH" git push >/dev/null 2>&1
    got=$?
    if [[ "$got" == "$1" ]]; then ok "$2"; else bad "$2 (want $1, got $got)"; fi
}
expect_git 0 "git -C dir push"                "git -C /repo push origin HEAD"
expect_git 0 "git -c k=v push"                "git -c user.name=x push origin HEAD"
expect_git 2 "git -c with a key off the allowlist" "git -c core.hooksPath=/dev/null push origin HEAD"
expect_git 0 "git --no-pager push"            "git --no-pager push"
expect_git 0 "git --git-dir=x push"           "git --git-dir=/repo/.git push"
expect_git 1 "git -C dir status"              "git -C /repo status"
expect_git 1 "git push-like subcommand"       "git pushx origin"

echo "Only data:"
expect 1 "different subcommand"               "gh pr view 2684"
expect 0 "echoed unquoted: every suffix is tried" "echo $T"
expect 0 "echoed as one quoted word is read as a script" "echo \"$T\""
expect 0 "quoted argument is read as a script"  "grep -n \"$T\" .claude/hooks/*.sh"
expect 1 "commit message"                     "git commit -m \"reminder on $T\""
expect 1 "heredoc written to a file"          "cat > f.md <<'EOF'${NL}run $T${NL}then wait${NL}EOF"
expect 1 "heredoc body line that is the command" "cat > f.md <<'EOF'${NL}$T --title x${NL}EOF"
expect 1 "heredoc fed to python"              "python3 - <<'PY'${NL}print('$T')${NL}PY"
expect 1 "tab-stripped heredoc"               "cat <<-EOF${NL}	$T${NL}	EOF"
expect 2 "block outside a git/gh message value" "echo \"\$(cat <<'EOF'${NL}$T${NL}EOF${NL})\""
expect 1 "longer word"                        "gh pr created"
expect 1 "reserved word as data"               "echo if then do $T-ish"
expect 1 "xargs running something else"         "echo x | xargs echo gh pr"
expect 1 "fd redirect on another command"       "ls 2>/dev/null"
expect 0 "single-quoted substitution text is read as a script" "echo '\$($T)'"

echo "A quoted word holding a command is a script, wherever it sits:"
expect 0 "ssh remote command"                   "ssh host '$T --title x'"
expect 0 "watch"                                "watch -g '$T'"
expect 0 "tmux"                                 "tmux new-session -d '$T'"
expect 0 "npx -c"                               "npx -c '$T'"
expect 0 "a variable's value"                   "X='$T' make"
expect 0 "an option's value after ="            "tool --run='$T'"
expect 0 "an attached short option's value"     "tool -x'$T'"
expect 0 "a ! helper value"                     "tool --helper='!$T'"
expect 0 "a script on a command, nested twice"  "ssh host \"bash -c '$T'\""
expect 0 "a word with ; and no space"           "tool 'true;$T'"
expect 2 "an unreadable word naming the command" "echo \"it's $T\""
expect 1 "an unreadable word naming nothing"     "echo \"it's fine\""
echo "...except a message value on git commit/tag or gh pr/issue:"
expect 1 "git commit -m"                        "git commit -m \"docs: $T\""
expect 1 "git commit --message="                "git commit --message=\"docs: $T\""
expect 1 "git commit -am bundle"                "git commit -am \"docs: $T\""
expect 1 "git commit attached -m"               "git commit -m\"docs: $T\""
expect 1 "git tag -m"                           "git tag -a v1 -m \"$T\""
expect 1 "gh issue create --title/--body"       "gh issue create --title 'x $T' --body '$T --base next'"
expect 1 "gh pr edit --body="                   "gh pr edit 5 --body='$T --base next'"
expect 1 "gh issue create -t/-b"                "gh issue create -t 'x $T' -b '$T --base next'"
expect 1 "gh -R before the group, --body"       "gh -R VATUSA/OIS issue create --title x --body '$T --base next'"
expect 1 "gh issue comment -b"                  "gh issue comment 5 -b '$T'"
expect 1 "git commit --subject is still a message option" "git commit --subject '$T'"
expect 0 "-m on a command that is not a message command" "tool -m '$T'"
expect 0 "--body on gh api"                     "gh api x --body '$T'"
expect 0 "a git commit editor value is not a message" "GIT_EDITOR='$T' git commit -m x"

echo "Gate fires (match or fail closed), never passes:"
gated "command after a heredoc inside \$()"     "x=\$(cat <<EOF${NL}y${NL}EOF${NL}$T)"
gated "heredoc fed to bash inside \$()"         "x=\$(bash <<'EOF'${NL}$T${NL}EOF${NL})"
gated "command after \$(cat <<'EOF')"           "x=\$(cat <<'EOF'${NL}b${NL}EOF${NL}); $T"
gated "heredoc opened on the \$() line"         "x=\$(cat <<'EOF')${NL}b${NL}EOF${NL}$T"
gated "sh -c heredoc inside \$()"               "x=\$(sh -c 'cat <<EOF${NL}b${NL}EOF${NL}$T${NL}EOF${NL}')"
gated "bash -c heredoc inside backticks"        "x=\`bash -c 'cat <<EOF${NL}b${NL}EOF${NL}$T${NL}EOF${NL}'\`"
gated "bash -c double-quoted heredoc in \$()"   "x=\$(bash -c \"cat <<EOF${NL}body${NL}EOF${NL}$T${NL}EOF${NL}\")"
gated "eval heredoc inside \$()"                "x=\$(eval \"cat <<EOF${NL}body${NL}EOF${NL}$T${NL}EOF${NL}\")"
gated "nested \$() heredoc then command"        "x=\$(y=\$(cat <<EOF${NL}z${NL}EOF${NL}); $T)"
gated "heredoc piped to bash inside \$()"       "x=\$(cat <<EOF | bash${NL}$T${NL}EOF${NL})"
gated "\$() in an unquoted body inside \$()"    "x=\$(cat <<EOF${NL}\$($T)${NL}EOF${NL})"
gated "\$() in an unquoted top-level body"      "cat >/dev/null <<EOF${NL}\$($T)${NL}EOF"
gated "backticks in an unquoted top-level body" "cat >/dev/null <<EOF${NL}\`$T\`${NL}EOF"
expect 0 "after );"                              "x=\$(true);$T"
expect 0 "after (true);"                         "(true);$T"
expect 0 "after &&("                             "true&&($T)"
expect 0 "after ;("                              "true;($T)"
expect 0 "after |("                              "true|($T)"
expect 0 "after ) and a newline"                 "(true)${NL}$T"
expect 0 "inside a brace group"                 "{ $T; }"
expect 0 "after if/then"                         "if true; then $T; fi"
expect 0 "after else"                            "if false; then true; else $T; fi"
expect 0 "in a while/do loop"                    "while false; do $T; done"
expect 0 "after until/do"                        "until true; do $T; done"
expect 0 "after !"                               "! $T"
expect 0 "after xargs"                           "echo x | xargs $T"
expect 0 "after xargs with options"              "echo x | xargs -n 1 -I {} $T"
expect 0 "after a leading fd redirect"           "2>/dev/null $T"
expect 0 "after a leading fd dup"                "2>&1 $T"
gated "output process substitution (fed by untraceable data)" "echo hi | tee >($T)"
gated "here-string fed to a shell via tee >(bash)" "tee >(bash) <<< 'gh pr create'"
gated "here-string piped to tee >(bash)" "cat <<< 'gh pr create' | tee >(bash)"
expect 0 "xargs -i (optional attached value)"   "echo x | xargs -i $T"
expect 0 "xargs -e (optional attached value)"   "echo x | xargs -e $T"
expect 0 "xargs -l (optional attached value)"   "echo x | xargs -l $T"
expect 0 "xargs -n 1 then a redirect"           "echo x | xargs -n 1 >f $T"
expect 0 "xargs -L 1 then an input redirect"    "echo x | xargs -L 1 </dev/null $T"
expect 0 "xargs --max-args 1"                   "echo x | xargs --max-args 1 $T"
expect 0 "BSD xargs -J"                         "echo x | xargs -J % $T"
expect 0 "coproc"                               "coproc $T"
expect 0 "function keyword"                     "function f { $T; }; f"
expect 0 "time -p"                              "time -p $T"
expect 0 "nice -n 5"                            "nice -n 5 $T"
expect 0 "sudo -u"                              "sudo -u x $T"
expect 0 "timeout 5"                            "timeout 5 $T"
expect 0 "env -u"                               "env -u X $T"
expect 0 "command -p"                           "command -p $T"
expect 0 "backslash-newline continuation"        "gh pr \\${NL}create"
expect 0 "bash -lc"                              "bash -lc '$T'"
expect 0 "sh -ec"                                "sh -ec '$T'"
expect 0 "bash -c -- script"                     "bash -c -- '$T'"
expect 0 "named fd redirect before the command"  "{fd}>/dev/null $T"
gated "apostrophe in a comment before the command" "# it's${NL}$T # '"
gated "apostrophe in a comment before a substitution" "# it's${NL}echo \$($T) # '"
gated "comment inside backticks hides a command" "echo \`echo #\`; $T"
gated "comment inside backticks, alone"          "echo \`echo #\`"
gated "backtick comment spanning two lines"      "echo \`echo #\`; $T${NL}echo \`echo #\`"
gated "ansi-c quoted command word" "g\$'it' push origin main"
gated "ansi-c quote containing a hash"           "echo \$'\\' #'; $T"
expect 1 "grep -c alias is not a git alias"      "grep -c alias.foo file.txt"
expect 1 "block after echo of the word function" "echo function x; git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
expect 1 "block after a paren in a quoted word"  "echo \"fix foo()\" && git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "here-string to a flock-launched shell" "flock /tmp/l bash <<< 'gh pr create'"
gated "heredoc pipe then ; operator" "cat <<'W' | bash; echo done
gh pr create
W"
gated "heredoc piped into a glued subshell" "cat <<'W' | (bash)
gh pr create
W"
gated "numbered GIT_CONFIG include.path" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=include.path GIT_CONFIG_VALUE_0=/tmp/e git p"
gated "heredoc piped into a subshell" "cat <<'W' | ( bash )
gh pr create
W"
gated "numbered GIT_CONFIG_KEY alias" "GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=alias.p GIT_CONFIG_VALUE_0='!gh pr create' git p"
expect 1 "flock with a non-shell final command" "flock /tmp/l grep x <<< 'gh pr create'"
gated "here-string piped into an if-compound" "cat <<< 'gh pr create' | if true; then bash; fi"
gated "heredoc piped into an if-compound" "cat <<'W' | if true; then bash; fi
gh pr create
W"
expect 1 "benign here-string into compound is data" "cat <<< 'echo hi there' | if true; then bash; fi"
gated "here-string piped to a shell" "cat <<< 'gh pr create' | bash"
gated "here-string piped through tee to a shell" "cat <<< 'gh pr create' | tee f.txt | bash"
expect 1 "here-string piped to grep is data" "cat <<< 'gh pr create' | grep x"
gated "here-string to bash" "bash <<< 'gh pr create'"
expect 1 "single-word here-string is not a heredoc" "bash <<<deploy"
expect 1 "single-word here-string with a space is not a heredoc" "bash <<< deploy"
gated "here-string to su" "su me <<< 'gh pr create'"
expect 1 "here-string to grep is data" "grep x <<< 'gh pr create'"
gated "heredoc read by script" "script -q /dev/null <<'W'
gh pr create
W"
gated "heredoc piped to a shell" "cat <<'W' | bash
gh pr create
W"
gated "runner with attached -c" "su me -c'gh pr create'"
gated "runner with --session-command=" "su me --session-command='gh pr create'"
gated "runner with --session-command sep" "runuser -u me --session-command 'gh pr create'"
gated "hash comment inside a substitution" "echo \"\$(echo a # )
gh pr create)\""
gated "case inside a substitution" "echo \"\$(case x in x) gh pr create;; esac)\""
gated "git -C then config alias"               "git -C /tmp/r config alias.ship '!gh pr create' && git -C /tmp/r ship"
expect 1 "attached --message= cat block is data" "git commit --message=\"\$(cat <<'W'
gh pr create
W
)\""
expect 1 "attached -m cat block is data"          "git commit -m\"\$(cat <<'W'
gh pr create
W
)\""
gated "git config sets a bang alias" "git config alias.p '!gh pr create' && git p"
gated "git -c include.path" "git -c include.path=/tmp/c p"
gated "GIT_CONFIG_GLOBAL before git" "GIT_CONFIG_GLOBAL=/tmp/c git p"
gated "git config alias set (fail closed)" "git config alias.co checkout"
gated "heredoc piped with |& to a shell" "cat <<'W' |& bash
gh pr create
W"
expect 1 "heredoc with || is not a pipe" "cat <<'W' || bash
gh pr create
W"
gated "heredoc piped to xargs shell -c" "cat <<'W' | xargs -0 bash -c
gh pr create
W"
gated "here-string to xargs shell -c" "xargs bash -c <<< 'gh pr create'"
gated "heredoc to a reader xargs shell -c" "xargs -0 bash -c <<'W'
gh pr create
W"
expect 1 "heredoc piped to xargs non-shell is data" "cat <<'W' | xargs rm
gh pr create
W"
gated "heredoc piped to flock-launched shell" "cat <<'W' | flock /tmp/l bash
gh pr create
W"
gated "here-string piped to flock-launched shell" "cat <<< 'gh pr create' | flock /tmp/l bash"
expect 1 "heredoc piped to flock non-shell is data" "cat <<'W' | flock /tmp/l rm
gh pr create
W"
gated "heredoc marker line \\-continues then pipes to a shell" "cat <<'W' \\
| bash
gh pr create
W
"
gated "heredoc read via tee output process substitution" "tee >(bash) <<'W'
gh pr create
W"
gated "heredoc redirected to output process substitution" "cat <<'W' > >(bash)
gh pr create
W"
gated "GIT_CONFIG_SYSTEM before git" "GIT_CONFIG_SYSTEM=/tmp/c git p"
gated "git -c includeIf path" "git -c includeIf.onbranch:main.path=/tmp/c p"
gated "comment ending in a backslash"            "# x \\${NL}$T"
gated "continuation before a #"                  "echo a\\${NL}#; $T"
gated "escaped space before a #"                 "echo \\ #; $T"
expect 1 "comment text is data"                  "echo hi # don't $T"
gated "git alias via -c"                         "git -c alias.p='!$T' p"
gated "git alias via -c, uppercase section"      "git -c Alias.p='!$T' p"
gated "git alias via --config-env="              "git --config-env=alias.p=X p"
gated "git alias via GIT_CONFIG_PARAMETERS"      "GIT_CONFIG_PARAMETERS=\"'alias.p=!$T'\" git p"
expect 1 "git -c with a non-alias key"           "git -c user.name=x commit -m hi"
gated "flock -c"                                 "flock /tmp/l -c '$T'"
gated "flock --command"                          "flock /tmp/l --command '$T'"
gated "su -c"                                    "su me -c '$T'"
gated "su --command="                            "su me --command='$T'"
gated "runuser -c"                               "runuser -l me -c '$T'"
gated "script -qc"                               "script -qc '$T' /dev/null"
gated "fish -c"                                  "fish -c '$T'"
gated "heredoc to su"                            "su me <<'W'${NL}$T${NL}W"
gated "heredoc to a shell behind a runner"       "flock /tmp/l bash <<'W'${NL}$T${NL}W"
expect 0 "grep -c pattern is read as a script"   "grep -c '$T' f.txt"
gated "block with a non-alias git -c override"  "git -c core.hooksPath=/tmp/h commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with git --config-env"             "git --config-env=core.hooksPath=H commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block after a function that shadows git"  "git() { eval \"\$3\"; }; git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block after a function keyword"           "function git { eval \"\$3\"; }; git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
expect 1 "parentheses in data before a block"    "echo \"done (1)\"; git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block in a plain assignment before git" "MSG=\"\$(cat <<'W'${NL}$T${NL}W${NL})\" git commit"
gated "block with GIT_EDITOR"                   "GIT_EDITOR=sh git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with EDITOR"                       "EDITOR=sh git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with GIT_CONFIG_PARAMETERS"        "GIT_CONFIG_PARAMETERS=x git commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with -e"                           "git commit -e -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with an abbreviated --ed"           "git commit --ed -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with bundled -ve"                  "git commit -ve -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with gh --editor"                  "gh issue create --editor --body \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with GH_EDITOR"                    "GH_EDITOR=sh gh issue create --body \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with --edit"                       "git commit --edit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "escaped backslash before a newline in quotes" "echo \"a\\\\${NL}\"; $T; echo \"b\\\\${NL}\""
expect 0 "continuation joins a command"          "gh pr cr\\${NL}eate"
expect 0 "backslash-newline in single quotes, read as a script" "echo 'gh pr cr\\${NL}eate'"
expect 2 "positional arg with an apostrophe after -c" "bash -c 'printf %s \"\$1\"' _ \"it's\""
gated "-c taken from an option value"          "bash --rcfile -xc -c '$T'"
gated "option value between -c and the script"  "bash -co pipefail '$T'"
gated "option between -c and the script"       "bash -c -e '$T'"
expect 0 "a chain of ten runners"                "nice nice nice nice nice nice nice nice nice nice $T"
expect 0 "behind the project lock wrapper"      "bash bin/with-lock.sh dusk --slots 4 -- $T"
expect 0 "find -exec"                           "find . -maxdepth 0 -exec $T \\;"
expect 0 "stdbuf"                               "stdbuf -oL $T"
expect 0 "flock"                                "flock /tmp/l $T"
expect 1 "env-prefixed commit form"               "env A=1 git commit -m \"\$(cat <<'EOF'${NL}msg (x)${NL}EOF${NL})\""
expect 1 "time-prefixed commit form"              "time git commit -m \"\$(cat <<'EOF'${NL}msg (x)${NL}EOF${NL})\""
expect 1 "timeout-prefixed commit form"           "timeout 600 git commit -m \"\$(cat <<'EOF'${NL}msg (x)${NL}EOF${NL})\""
expect 1 "timeout -s KILL commit form"            "timeout -s KILL 600 git commit -m \"\$(cat <<'EOF'${NL}msg (x)${NL}EOF${NL})\""
expect 1 "commit form inside if/then"            "if true; then git commit -m \"\$(cat <<'EOF'${NL}msg${NL}EOF${NL})\"; fi"
expect 1 "commit form after a leading fd redirect" "2>/dev/null git commit -m \"\$(cat <<'EOF'${NL}msg${NL}EOF${NL})\""
expect 1 "commit form after );"                  "x=\$(true);git commit -m \"\$(cat <<'EOF'${NL}msg${NL}EOF${NL})\""
gated "quoted ) in an unquoted body's \$()"     "cat >/dev/null <<EOF${NL}\$(echo ')'; $T)${NL}EOF"
gated "double-quoted ) in an unquoted body"     "cat >/dev/null <<EOF${NL}\$(echo \")\"; $T)${NL}EOF"
gated "case in an unquoted body's \$()"        "cat >/dev/null <<EOF${NL}\$(case a in a) $T;; esac)${NL}EOF"
gated "single-quoted \$() in an unquoted body"  "cat >f <<EOF${NL}'\$($T)'${NL}EOF"
gated "single-quoted backticks in a body"      "cat >f <<EOF${NL}'\`$T\`'${NL}EOF"
gated "apostrophes around \$() in a body"      "cat >f <<EOF${NL}it's \$($T) isn't${NL}EOF"
gated "backtick in a quoted body inside backticks" "x=\`cat <<'EOF'${NL}\` ; $T ; \`${NL}EOF${NL}\`"
gated "EOF) closes the \$() early"             "x=\$(cat <<'EOF'${NL}b${NL}EOF)${NL}$T${NL}EOF${NL})"
gated "\$(cat <<'EOF') inside single quotes"    "echo '\$(cat <<'EOF'${NL}$T${NL}EOF${NL})'"
gated "a comment before the quoted block"       "true # \$(cat <<'EOF'${NL}$T${NL}EOF${NL})"
gated "quoted block inside an outer cat heredoc" "cat <<\"A\"${NL}\$(cat <<\"B\"${NL}A${NL}$T${NL}B${NL})${NL}A"
gated "quoted block inside an outer bash heredoc" "bash <<A${NL}\$(cat <<\"B\"${NL}A${NL}$T${NL}B${NL})${NL}A"
gated "\${ cmd; } in an unquoted body"          "cat >/dev/null <<EOF${NL}\${ $T; }${NL}EOF"
gated "\${| cmd; } in an unquoted body"         "cat >/dev/null <<EOF${NL}\${| $T; }${NL}EOF"
gated "block as the command word"              "\$(cat <<'W'${NL}$T${NL}W${NL})"
gated "block given to eval"                    "eval \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block given to bash -c"                 "bash -c \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block given to sh -c"                   "sh -c \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block after env"                        "env \$(cat <<'W'${NL}$T${NL}W${NL})"
gated "block after sudo"                       "sudo \$(cat <<'W'${NL}$T${NL}W${NL})"
gated "block piped to bash"                    "echo \"\$(cat <<'W'${NL}$T${NL}W${NL})\" | bash"
gated "block piped to xargs"                   "echo \$(cat <<'W'${NL}$T${NL}W${NL}) | xargs"
gated "block stored then run"                  "x=\$(cat <<'W'${NL}$T${NL}W${NL}); \$x"
gated "block as an unlisted option"            "git commit -F \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block as -m of a non-git program"       "echo -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block to an unknown git subcommand"     "git myalias -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block with an inline git alias"         "git -c alias.ci=!sh commit -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block to an unknown gh subcommand"      "gh myext --body \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block to a non-git program after sudo"  "sudo echo -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
expect 1 "block as gh issue comment body"      "gh issue comment 1 --body \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
expect 1 "block as git tag message"            "git tag -a v1 -m \"\$(cat <<'W'${NL}$T${NL}W${NL})\""
gated "block in an env assignment before git"  "GIT_EDITOR=\"\$(cat <<'W'${NL}$T${NL}W${NL})\" git commit"
gated "block in an outer heredoc (quoted form)" "cat <<\"A\"${NL}\"\$(cat <<\"B\"${NL}A${NL}$T${NL}B${NL})\"${NL}A"
gated "EOF) ends the block (quoted form)"      "git commit -m \"\$(cat <<'EOF'${NL}b${NL}EOF)\" ; $T${NL}EOF${NL})\""

echo "gh aliases and group flags before the subcommand:"
expect 0 "the pr new alias"                   "gh pr new --base next --title x"
expect 0 "-R before the subcommand"           "gh pr -R VATUSA/OIS create --title x"
expect 0 "--repo= before the subcommand"      "gh pr --repo=VATUSA/OIS new --title x"
expect 0 "-R attached before the subcommand"  "gh pr -RVATUSA/OIS create"
# gh also takes the repo flag before the group (`gh -R o/r pr list` runs), so it moves too.
expect 0 "-R before the group"                "gh -R VATUSA/OIS pr create --title x"
expect 0 "--repo before the group, pr new"    "gh --repo VATUSA/OIS pr new --title x"
expect 0 "--repo= before the group"           "gh --repo=VATUSA/OIS pr create"
expect 0 "-R attached before the group"       "gh -RVATUSA/OIS pr create"
expect 1 "-R before another group"            "gh -R VATUSA/OIS issue create --title x"
expect 1 "gh api with -X before the endpoint" "gh api -X PUT repos/o/r/pulls/5/update-branch"
expect 1 "gh pr new-ish word"                 "gh pr newest"
expect 2 "gh alias set on the line"           "gh alias set pc 'pr create' && gh pc"
expect 2 "gh alias import on the line"        "gh alias import aliases.yml && gh pc"

echo "env -S runs its script, so a shell in it reads stdin:"
expect 0 "env -S bash here-string"              "env -S bash <<< '$T'"
expect 0 "env -Sbash here-string"               "env -Sbash <<< '$T'"
expect 0 "env -S bash heredoc"                  "env -S bash <<'W'${NL}$T${NL}W"
expect 0 "heredoc piped to env -S bash"         "cat <<'W' | env -S bash${NL}$T${NL}W"
expect 0 "env --split-string=bash here-string"  "env --split-string=bash <<< '$T'"
expect 0 "env --split-string script"            "env --split-string='$T'"
expect 0 "env -S 'bash -s' here-string"         "env -S 'bash -s' <<< '$T'"
expect 1 "env -S cat heredoc is data"           "env -S cat <<'W'${NL}$T${NL}W"
expect 2 "env -S with an unbalanced quote"      "env -S \"bash 'x\" <<< '$T'"

echo "ANSI-C quoting fails closed only where a command could run the target:"
expect 2 "\$'...' on the target's line"          "gh \$'pr' create"
expect 2 "\$'...' as the command"                "\$'gh' pr create"
expect 2 "\$'...' anywhere on a line running gh" "IFS=\$'\\n'; gh pr view 1"
expect 2 "\$'...' in an assignment then eval"    "x=\$'gh pr create'; eval \"\$x\""
expect 1 "\$'...' on a line with no gh"          "IFS=\$'\\n'; for f in a b; do echo \"\$f\"; done"
expect 2 "unclosed \$'"                          "echo \$'abc"

echo "Unparseable fails closed:"
expect 2 "unbalanced quote"                   "echo \"$T"
expect 2 "unterminated heredoc"               "cat <<EOF${NL}$T"
expect 2 "unquoted heredoc in \$() (quote it)"  "x=\$(cat <<EOF${NL}b${NL}EOF${NL})"

echo "Cost stays linear:"
for shape in "x2000:bash" "bash -c" "eval eval eval eval eval eval eval eval" "xargs" "nice -n 5 a b c d e nice -n 5 a b c d e nice -n 5 a b c d e nice -n 5 a b c d e nice -n 5 a b c d e nice -n 5 a b c d e"; do
    secs="$(python3 - "$MATCH" "$shape" <<'PY'
import importlib.util, sys, time
spec = importlib.util.spec_from_file_location("cm", sys.argv[1])
cm = importlib.util.module_from_spec(spec); spec.loader.exec_module(cm)
shape = sys.argv[2]
line = " ".join([shape[6:]] * 2000) if shape.startswith("x2000:") else shape + " " + " ".join("w%d" % i for i in range(2000))
start = time.time()
try:
    cm.invokes(line, ["gh", "pr", "create"])
except cm.Unparseable:
    pass
print("%.2f" % (time.time() - start))
PY
)"
    if python3 -c "import sys; sys.exit(0 if float(sys.argv[1]) < 0.5 else 1)" "$secs"; then ok "2,000-word line after '${shape:0:12}…' parses in ${secs}s"; else bad "2,000-word line after '${shape:0:12}…' took ${secs}s"; fi
done

echo "A word is read as the shell leaves it:"
expect 0 "zsh =cmd is cmd"                        "=gh pr create --base next"
expect 0 "=cmd behind a runner's option"          "sudo -n =gh pr create"
expect 0 "zsh precommand modifiers"               "noglob nocorrect - $T"
expect 0 "a case body runs"                       "case \$x in a|b*) $T;; esac"
expect 1 "=cmd for another command"               "=ls pr create"
expect 0 "a substitution inside arithmetic runs"  'echo $(( $(gh pr create) + 1 ))'
expect 0 "an arithmetic-looking substitution that does not balance is a subshell" 'echo $((gh pr create); echo x)'
expect 0 "...even when it happens to end in two parentheses" 'echo $((gh pr create); (echo x))'
expect 0 "a bare ((cmd)) is nested subshells in dash" '((gh pr create))'
echo "A built word that is an argument, arithmetic, a test or a case pattern is data:"
for line in 'echo "$greeting world"' 'cp "$a" "$b"' 'mv $src $dst' 'for f in $list; do echo "$f"; done' \
    'case $x in *) echo y;; esac' 'case $x in a|b*|?) echo y;; esac' '[ -f a ] && echo y' '[[ $a == b* ]] || echo n' \
    'x=$(date) && echo "$x"' 'cp a.{txt,bak}' 'ls *.md' 'ls *(.)' 'echo $((1+(2*3)))' 'gh pr view $n' \
    'echo $(($x * 2)) "$(( $a + $b ))"' '((i++)); echo $i' 'for ((i=0; i<3; i++)); do echo $i; done' \
    'gh pr view "$n" --json title' 'gh -R "$repo" pr view 5' 'echo ==== ; ls' '[ a = b ] || echo n' \
    "echo '{\"a\":1,\"b\":\"x y\"}'" 'find . -name "*.php" -exec grep -l x {} +' 'f() { echo hi; }; f' \
    'git -C "$dir" status' 'IFS=, read -r a b <<< "$line"' 'echo "$x" | xargs -I{} echo {}'; do
    expect 1 "data: $line" "$line"
done

echo "Usage:"
printf '%s' "$T" | python3 "$MATCH" >/dev/null 2>&1
status=$?
if (( status == 2 )); then ok "no target words is a usage error"; else bad "no target words is a usage error (got $status)"; fi

echo ""
echo "command-match: $PASS passed, $FAIL failed"
(( FAIL == 0 ))
