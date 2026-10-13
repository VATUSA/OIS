"""What attribution-gate.sh must scan on a Bash line: whether it posts text, the message files it reads,
and every word, each on its own.

  printf '%s' "$COMMAND" | python3 -I attribution_targets.py CWD > records

The line is parsed with command_match.py, as every gate here is, so a heredoc or quoted argument that only
names `git commit` is data. It watches `git commit`; `gh pr create|edit|comment|review|close|reopen|merge`
and `gh issue create|comment|edit|close|reopen` (and gh's `new` alias), wherever -R sits; and every
`gh api` call that writes (git_policy.api_calls: a method other than GET/HEAD, or fields or --input with
no method), whatever its endpoint, since issue and comment bodies, PR review comments and GraphQL
mutations all travel that way.

Prints NUL-terminated records: `W` when the line runs one of those; `A` when one is a gh api write;
`F<path>` for each file a message is read from (git commit -F/--file, gh --body-file/-F, gh api
-F key=@file and --input), made absolute against CWD, or against DIR for `cd DIR && ...`; `X<reason>` when
a relative message file sits on a line that moves directory some other way; and `Q<word>` for every word
the line holds, at every depth (bash -c scripts, substitutions, a heredoc a shell reads), plus each
double-quoted string inside a GraphQL mutation, so a body's last line ends where its word does, and the value
of every `name=value` word on its own (`-f body=X`, `--body=X`), so a footer that is the whole body starts a line.

Exit 0 after printing; 2 with the reason on stderr when the line cannot be parsed, which the gate blocks.
Python 3.8-compatible.
"""

import os
import re
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from command_match import (  # noqa: E402
    Unparseable, cd_target, command_strings, drop_quoted_cat_blocks, invocations, join_and_strip, shell_scripts,
    shell_words, simple_commands, split_heredocs, substitutions)
from git_policy import API_LONG_VALUES, API_SHORT_FLAGS, API_SHORT_VALUES, api_calls, moves_repository  # noqa: E402

# Short options that take no value, which may sit in a bundle before -F: `git commit -aF msg`, `gh pr create -dF b`.
BOOLEAN_LETTERS = {"git": set("asvnqeuipz"), "gh": set("dfwe")}
# `name=value`, `-name=value` or `--name=value`: the value, for scanning on its own.
OPTION_VALUE = re.compile(r"^-{0,2}[A-Za-z_][A-Za-z0-9_.\[\]-]*=(.*)$", re.DOTALL)
GH_TEXT_COMMANDS = [["gh", "pr", sub] for sub in ("create", "edit", "comment", "review", "close", "reopen", "merge")] \
    + [["gh", "issue", sub] for sub in ("create", "comment", "edit", "close", "reopen")]


def option_files(args, short, long, booleans):
    """The values of `-<short> X`, `-<short>X`, `--<long> X` and `--<long>=X` in ARGS. In a short bundle the
    letter counts when only BOOLEANS come before it (`-aF msg`), and takes the rest of the bundle or the next
    word; `-tFix` is a title."""
    found, index = [], 0
    while index < len(args):
        arg = args[index]
        index += 1
        if arg == "--":
            break
        if arg in long:
            found.append(args[index] if index < len(args) else "")
            index += 1
        elif any(arg.startswith(name + "=") for name in long):
            found.append(arg.split("=", 1)[1])
        elif re.match(r"-[A-Za-z]", arg) and short in arg[1:] and set(arg[1:arg.index(short, 1)]) <= booleans:
            rest = arg[arg.index(short, 1) + 1:]
            if rest:
                found.append(rest)
            else:
                found.append(args[index] if index < len(args) else "")
                index += 1
    return found


def api_files(args):
    """The files a gh api call reads its body from: `-F key=@file` (a raw -f field never reads one) and
    --input; `@-` and `-` are stdin."""
    found, index = [], 0
    while index < len(args):
        arg = args[index]
        index += 1
        if arg == "--":
            break
        if arg.startswith("--"):
            name, equals, value = arg.partition("=")
            if name in API_LONG_VALUES and not equals:
                value = args[index] if index < len(args) else ""
                index += 1
            if name == "--input":
                found.append(value)
            elif name == "--field" and "=@" in value:
                found.append(value.split("=@", 1)[1])
        elif arg.startswith("-") and len(arg) > 1:
            for position, letter in enumerate(arg[1:], 1):
                if letter in API_SHORT_VALUES:
                    value = arg[position + 1:]
                    if not value:
                        value = args[index] if index < len(args) else ""
                        index += 1
                    elif value.startswith("="):
                        value = value[1:]
                    if letter == "F" and "=@" in value:
                        found.append(value.split("=@", 1)[1])
                    break
                if letter not in API_SHORT_FLAGS:
                    raise Unparseable("gh api flag `%s` is not one the gate knows" % arg)
    return found


def mutation_literals(word):
    """Each double-quoted string literal in a GraphQL mutation word, unescaped as GraphQL reads `\\"`."""
    if "mutation" not in word:
        return []
    return [re.sub(r'\\(["\\])', r"\1", literal) for literal in re.findall(r'"((?:[^"\\]|\\.)*)"', word)]


def words_of(text, depth=0):
    """Every word TEXT holds, at every depth a shell would read it."""
    if depth > 8:
        return []
    try:
        code, shell_bodies = split_heredocs(drop_quoted_cat_blocks(text))
        code = join_and_strip(code)
        groups = simple_commands(code)
        nested = shell_bodies + substitutions(code)
    except Unparseable:
        return []
    found = []
    for raw, herestrings, _operator in groups:
        words = shell_words(raw)
        found += words + herestrings
        nested += shell_scripts(words) + command_strings(words)
    for word in list(found):
        found += mutation_literals(word)
    for inner in dict.fromkeys(nested):
        found += words_of(inner, depth + 1)
    return found


def main():
    cwd = sys.argv[1] if len(sys.argv) > 1 and os.path.isdir(sys.argv[1]) else os.getcwd()
    text = sys.stdin.read()
    records, files, watched, api = [], [], False, False
    try:
        for args in invocations(text, ["git", "commit"]):
            watched = True
            files += [("git", "commit", path) for path in option_files(args, "F", ("--file",), BOOLEAN_LETTERS["git"])]
        for target in GH_TEXT_COMMANDS:
            for args in invocations(text, target):
                watched = True
                files += [tuple(target[:2]) + (path,)
                          for path in option_files(args, "F", ("--body-file",), BOOLEAN_LETTERS["gh"])]
        for _endpoint, method, args in api_calls(text):
            if method in ("GET", "HEAD"):
                continue
            watched = api = True
            files += [("gh", "api", path) for path in api_files(args)]
        moved = watched and moves_repository(text)
    except Unparseable as error:
        print("attribution_targets: %s" % error, file=sys.stderr)
        return 2
    if not watched:
        return 0
    records.append("W")
    if api:
        records.append("A")
    for tool, sub, path in files:
        if path in ("-", "/dev/stdin", "/dev/fd/0") or os.path.isabs(path) or re.search(r"[$`]", path):
            records.append("F" + path)
            continue
        base = cwd
        if moved:
            base = cd_target(text, [tool, sub]) if tool == "gh" else cd_target(text, ["git", sub])
            if not base:
                records.append("X%s reads %s, a relative path on a line that changes directory or repository; "
                               "pass an absolute path, or run it as `cd DIR && %s %s ...`" % (
                                   " ".join((tool, sub)), path, tool, sub))
                continue
        records.append("F" + os.path.normpath(os.path.join(base, os.path.expanduser(path))))
    words = list(dict.fromkeys(words_of(text)))
    # A value attached to its name (`-f body=X`, `--body=X`, `-F key=X`) is a body of its own: with the name
    # in front, a footer that is the whole body never starts its line, so the value is scanned alone too.
    for word in list(words):
        match = OPTION_VALUE.match(word)
        while match:  # `--raw-field=body=X` names twice
            words.append(match.group(1))
            match = OPTION_VALUE.match(match.group(1))
    records += ["Q" + word for word in dict.fromkeys(words) if word.strip()]
    sys.stdout.write("".join(record.replace("\0", "") + "\0" for record in records))
    return 0


if __name__ == "__main__":
    sys.exit(main())
