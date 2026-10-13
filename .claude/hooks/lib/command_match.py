"""Does a shell command line run a given command? Used by hooks that gate or remind on one.

  printf '%s' "$COMMAND" | python3 command_match.py gh pr create

Exit 0 when some part of the line would run it, 1 when it would not, 2 when the line
cannot be parsed or is refused (or no words were given), with the reason on stderr. A gate
treats 2 as a match and fails closed.

A substring check fires on data: a heredoc written to a file, a commit message. This parses
instead. Comments are dropped, and heredoc bodies unless a shell reads them; -c scripts of
shells and runners (flock, su, script), eval, $(...), backticks, <(...) and env -S are parsed
as commands too, and every suffix of a command is tried, so any runner in front (sudo, xargs,
a wrapper script) is seen. Any other word holding whitespace or ; & | (an ssh, watch or tmux
command, `rebase --exec`, `-c core.fsmonitor=...`, a variable's value) is parsed as a script
as well, except a message value (-m, --message, --body, --title) on git commit/tag or gh
pr/issue; such a word the parser cannot read fails closed only when it names the command, as a
grep would. A git exec-path binary (`git-push`) or a symlink to git counts as git, and so does
zsh's `=git` (the path of git).

Refused on any line: `git config` writing a key off WRITABLE_CONFIG (and so -c, --config-env
and GIT_CONFIG_KEY_<n>; core.sshCommand and GIT_SSH_COMMAND only as plain ssh), its --edit and
section renames, GIT_CONFIG_PARAMETERS/GLOBAL/SYSTEM,
`git remote add --mirror`, the plumbing pushes (send-pack, http-push) and `gh alias set` or
`import`; a push, merge, rebase or pull on a line that assigns or unsets HOME, XDG_CONFIG_HOME or
GIT_CONFIG_GLOBAL/SYSTEM; ANSI-C `$'...'` quoting on a line that could run the command.
A static parser cannot see a command built at runtime ($x push, a shell alias or function,
config written earlier, text piped to a shell, a copy of git under another name); the gates
are a guard rail, not a sandbox.
Python 3.8-compatible.
"""

import functools
import os
import re
import shlex
import sys

SHELLS = {"bash", "sh", "zsh", "dash", "ksh", "ash", "mksh", "yash", "fish", "csh", "tcsh"}
# Run a -c/--command string, or start a shell that reads stdin (su, runuser, script).
RUNNERS = {"flock", "su", "runuser", "script"}
STDIN_SHELLS = SHELLS | {"su", "runuser", "script"}
# Build a command line from stdin/args, so a shell among their words runs the piped data as code.
ARG_BUILDERS = {"xargs"}
WRAPPERS = {"command", "builtin", "exec", "time", "nohup", "sudo", "nice", "noglob", "nocorrect", "-"}
RESERVED_WORDS = {"{", "}", "!", "if", "then", "else", "elif", "do", "while", "until"}
COMPOUND_OPENERS = {"if", "while", "until", "case", "{", "("}  # a pipe into one of these may reach a shell inside it
SEPARATORS = {";", "&", "&&", "|", "||", "|&", ";;", "(", ")", "\n"}
REDIRECTS = {"<", ">", ">>", "<<", "<<<", "<>", ">&", "<&", ">|", "&>", "&>>"}
HEREDOC_RE = re.compile(r"(?<!<)<<(?!<)(-?)\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\2")  # not <<< (a here-string)
# The project's commit/PR-body form: `"$(cat <<'EOF'`, a body, `EOF` alone, then `)"`.
QUOTED_CAT_BLOCK_RE = re.compile(r"\"\$\(cat <<(['\"])([A-Za-z_][A-Za-z0-9_]*)\1\n(.*?)\n\2\n\)\"", re.DOTALL)
BLOCK = "QUOTED_CAT_BLOCK"
ANSI_C = "\x01"  # stands in for a `$'...'` word part (see join_and_strip)
FUNCTION_RE = re.compile(r"(?:^|[\s;&|(])(?:function\s+(?:git|gh)\b|(?:git|gh)\s*\(\s*\))")
BLOCK_OPTIONS = {"-m", "--message", "--body", "--title"}
BLOCK_COMMANDS = {("git", "commit"), ("git", "tag"), ("gh", "pr", "create"), ("gh", "pr", "edit"),
                  ("gh", "pr", "comment"), ("gh", "issue", "create"), ("gh", "issue", "edit"),
                  ("gh", "issue", "comment")}
OPERATORS = sorted(SEPARATORS | REDIRECTS, key=len, reverse=True)


class Unparseable(Exception):
    pass


class Refused(Unparseable):
    """A policy refusal (saved config, an alias, nesting too deep): it stands wherever the text came from,
    even inside a quoted argument, where a mere parse failure does not."""


def drop_quoted_cat_blocks(text):
    """Replace each exact `"$(cat <<'EOF' … EOF\\n)"` block with the word BLOCK (see check_blocks).

    Only where nothing before it could change how a shell reads it (a quote, backtick, backslash,
    comment, earlier heredoc or a function that could shadow git/gh), and where no body line starts
    with the delimiter (bash also ends the body at `EOF)`).
    Anything else is left for the general parser, which fails closed on a heredoc in a substitution.
    """
    while True:
        match = QUOTED_CAT_BLOCK_RE.search(text)
        if not match or re.search(r"['`\\#<]", text[:match.start()]) or FUNCTION_RE.search(text[:match.start()]):
            return text
        word, body = match.group(2), match.group(3)
        if any(line.startswith(word) for line in body.split("\n")):
            return text
        text = text[:match.start()] + BLOCK + text[match.end():]


def expand_attached_options(command):
    """Split an attached message value (`--body=BLOCK`, `-mBLOCK`) into option + value, but only when it
    carries a BLOCK, so `check_blocks` sees the same shape as the separate form."""
    out = []
    for word in command:
        if BLOCK not in word:
            out.append(word)
            continue
        long = re.match(r"(--message|--body|--title)=(.*)$", word, re.DOTALL)
        short = re.match(r"(-m)(.+)$", word, re.DOTALL)
        if long:
            out += [long.group(1), long.group(2)]
        elif short:
            out += [short.group(1), short.group(2)]
        else:
            out.append(word)
    return out


def check_blocks(words):
    """A block's text is data only as the value of a message option on a known git/gh subcommand."""
    if not any(BLOCK in word for word in words):
        return
    command = strip_prefix(words)
    prefix = words[:len(words) - len(command)]
    if any(re.match(r"(GIT_[A-Z_]*|GH_[A-Z_]*|EDITOR|VISUAL)=", word) for word in prefix):
        raise Unparseable("editor or git config set alongside a quoted cat block")
    if any(opens_editor(word) for word in command):
        raise Unparseable("editor alongside a quoted cat block")
    if command and os.path.basename(command[0]) == "git":
        if any(word in ("-c", "--config-env") or word.startswith("--config-env=") for word in command):
            raise Unparseable("git config override alongside a quoted cat block")
        command = skip_git_options(command)
    command = expand_attached_options(command)
    positions = [i for i, word in enumerate(command) if BLOCK in word]
    known = bool(command) and any(
        [os.path.basename(command[0])] + command[1:len(key)] == list(key) for key in BLOCK_COMMANDS)
    if (sum(BLOCK in word for word in words) != len(positions) or not known
            or any(command[i] != BLOCK or command[i - 1] not in BLOCK_OPTIONS for i in positions)):
        raise Unparseable("a quoted cat block is not a git/gh message value")


def opens_editor(word):
    """-e in a short-option bundle, or any abbreviation of --edit/--editor (git and gh accept both)."""
    option = word.split("=", 1)[0]
    return (bool(re.fullmatch(r"-[A-Za-z]+", option)) and "e" in option) or (
        len(option) > 2 and "--editor".startswith(option))


def split_heredocs(text):
    """Return (text without heredoc bodies, bodies read by a shell)."""
    lines = text.split("\n")
    kept, shell_bodies, index = [], [], 0
    while index < len(lines):
        line = lines[index]
        kept.append(line)
        index += 1
        for match in HEREDOC_RE.finditer(line):
            if quoted_or_commented("\n".join(kept[:-1] + [line[:match.start()]])):
                raise Unparseable("heredoc marker inside a quote or comment")
            if re.search(r"[<>]\(", line):
                # `tee >(bash) <<'W'` / `cat <<'W' > >(bash)` feed the body to a shell we can't trace.
                raise Unparseable("heredoc command uses process substitution")
            if (len(line) - len(line.rstrip("\\"))) % 2 == 1:
                # `cat <<'W' \` continues onto the next line, so the pipe/reader is not yet visible here.
                raise Unparseable("heredoc marker line ends with a line-continuation")
            strip_tabs, word = match.group(1) == "-", match.group(3)
            body = []
            while True:
                if index >= len(lines):
                    raise Unparseable("heredoc %s is never closed" % word)
                current = lines[index]
                index += 1
                if (current.lstrip("\t") if strip_tabs else current) == word:
                    break
                body.append(current.lstrip("\t") if strip_tabs else current)
            body_text = "\n".join(body)
            if reader_is_shell(line[:match.start()]) or pipes_to_shell(line[match.end():]):
                shell_bodies.append(body_text)
            elif not match.group(2) and re.search(r"\$\(|`|\$\{[\s|]", body_text):
                # An unquoted body runs its substitutions, and its quotes are literal: too subtle to parse.
                raise Unparseable("unquoted heredoc %s runs a command substitution" % word)
    return "\n".join(kept), shell_bodies


def pipes_to_shell(after):
    """A heredoc piped on the same line to a shell (`cat <<'W' | bash`)."""
    for segment in re.split(r"\|&|(?<!\|)\|(?!\|)", after)[1:]:  # split on | and |&, not || (which does not pipe)
        segment = re.split(r"[;&\n]|\|\|", segment)[0]  # the pipe destination ends at ;/&/&&/||/newline
        try:
            raw = shlex.split(segment)
        except ValueError:
            return True
        if raw and (raw[0] in COMPOUND_OPENERS or raw[0].startswith("(")):  # `| if …; fi`, `| (bash)`
            return True
        words = strip_prefix(raw)
        if words and os.path.basename(words[0]) in STDIN_SHELLS:
            return True
        if words and os.path.basename(words[0]) in RUNNERS | ARG_BUILDERS and any(
                os.path.basename(word) in STDIN_SHELLS for word in words[1:]):
            return True  # `| flock … bash`, `| xargs bash -c` runs the piped heredoc body as a shell script
    return False


def quoted_or_commented(prefix):
    """Whether text ending here is inside a quote or a comment, so a `<<` there is not a heredoc."""
    quote, comment, index = None, False, 0
    while index < len(prefix):
        char = prefix[index]
        if comment:
            comment = char != "\n"
        elif quote == "'":
            quote = None if char == "'" else quote
        elif char == "\\":
            index += 1
        elif quote == '"':
            quote = None if char == '"' else quote
        elif char in "'\"":
            quote = char
        elif char == "#" and (index == 0 or prefix[index - 1] in " \t\n;&|("):
            comment = True
        index += 1
    return quote is not None or comment


def reader_is_shell(before):
    """Whether a shell, su, runuser or script in the command owning a heredoc may read it as a script."""
    segment = re.split(r"&&|\|\||[;|&(]", before)[-1]
    try:
        words = strip_prefix(shlex.split(segment))
    except ValueError:
        return True
    return any(os.path.basename(word) in STDIN_SHELLS for word in words)


def substitutions(text):
    """The contents of $(...), backticks and <(...) outside single quotes."""
    found, index, quote = [], 0, None
    while index < len(text):
        char = text[index]
        if char == "\\" and quote != "'":
            index += 2
            continue
        if quote == "'":
            if char == "'":
                quote = None
        elif char == "'" and quote is None:
            quote = "'"
        elif char == '"':
            quote = None if quote == '"' else '"'
        elif char == "`":
            end = text.find("`", index + 1)
            if end == -1:
                raise Unparseable("unclosed backtick")
            found.append(text[index + 1:end])
            index = end
        elif text.startswith("$(", index) or (quote is None and text.startswith("<(", index)):
            depth, start, inner = 1, index + 2, None
            index = start
            while index < len(text) and depth:
                current = text[index]
                if inner == "'":
                    inner = None if current == "'" else inner
                elif current == "\\":
                    index += 2
                    continue
                elif inner == '"':
                    inner = None if current == '"' else inner
                elif current in "'\"":
                    inner = current
                elif current == "#" and (index == start or text[index - 1] in " \t\n;&|("):
                    newline = text.find("\n", index)  # `#` comments even inside "$(…)"; skip it so parens still match
                    index = len(text) if newline == -1 else newline
                    continue
                else:
                    depth += {"(": 1, ")": -1}.get(current, 0)
                index += 1
            if depth:
                raise Unparseable("unclosed command substitution")
            body = text[start:index - 1]
            if re.search(r"(?:^|[\s;&|(])case\s", body):  # `case` patterns use `)`, which breaks paren counting
                raise Unparseable("a case statement inside a command substitution")
            found.append(body)
            continue
        index += 1
    return found


def split_operators(token):
    """shlex joins adjacent punctuation (`);`, `&&(`, `)` + newline): split it into shell operators."""
    if not token or token in SEPARATORS or token in REDIRECTS or set(token) - set("();<>|&\n"):
        return [token]
    parts = []
    while token:
        operator = next(op for op in OPERATORS if token.startswith(op))
        parts.append(operator)
        token = token[len(operator):]
    return parts


def join_and_strip(text):
    """Drop backslash-newline continuations and `#` comments, minding quotes, as the shell reads them.

    Tracks single, double and backtick state. `$'…'` (ANSI-C) quoting becomes the ANSI_C placeholder, which
    invocation_details refuses wherever a command could run it: a static parser cannot reproduce its quote
    removal. A `#` opening a comment inside backticks is too ambiguous, so it fails closed (Unparseable)."""
    out, index, quote, backtick = [], 0, None, False
    boundary = set(" \t\n;&|()<>")
    while index < len(text):
        char = text[index]
        if quote == "'":
            if char == "'":
                quote = None
            out.append(char)
            index += 1
        elif char == "\\":
            if text[index + 1:index + 2] != "\n":  # a line continuation joins the lines; keep any other escape
                out.append(text[index:index + 2])
            index += 2
        elif char == "$" and text[index + 1:index + 2] == "'" and quote is None:
            end = index + 2
            while end < len(text) and text[end] != "'":
                end += 2 if text[end] == "\\" else 1
            if end >= len(text):
                raise Unparseable("unclosed ANSI-C $'...' quoting")
            out.append(ANSI_C)  # text a static parser cannot expand; refused where a command could run it
            index = end + 1
        elif char == "'" and quote is None:
            quote = "'"
            out.append(char)
            index += 1
        elif char == '"':
            quote = None if quote == '"' else '"'
            out.append(char)
            index += 1
        elif char == "`":
            backtick = not backtick
            out.append(char)
            index += 1
        elif char == "#" and (not out or out[-1] in boundary):
            if backtick:
                raise Unparseable("a comment inside backticks")
            if quote is None:
                end = text.find("\n", index)  # a comment ends at the newline; a backslash in it continues nothing
                index = len(text) if end == -1 else end
                continue
            out.append(char)
            index += 1
        else:
            out.append(char)
            index += 1
    return "".join(out)


def simple_commands(text):
    """Split a line into argv lists at shell separators, dropping redirections."""
    text = re.sub(r"(?<![^\s;&|()])(\d+|\{[A-Za-z_]\w*\})(?=[<>])", "", text)  # the fd of `2>`/`{fd}>`, else a word
    lexer = shlex.shlex(text, posix=True, punctuation_chars="();<>|&\n")
    lexer.whitespace = " \t\r"
    lexer.whitespace_split = True
    lexer.commenters = ""
    try:
        tokens = [part for token in lexer for part in split_operators(token)]
    except ValueError as error:
        raise Unparseable(str(error))
    commands, heres, ops, current, current_here = [], [], [], [], []
    skip_next, capture_here = False, False
    for token in tokens:
        if skip_next:
            skip_next = False
            if capture_here:
                current_here.append(token)  # the here-string word after `<<<`
                capture_here = False
            continue
        if token in SEPARATORS:
            commands.append(current)
            heres.append(current_here)
            ops.append(token)  # the operator that ends this command (`|`, `&&`, `;`, …)
            current, current_here = [], []
        elif token == "<<<":
            skip_next = capture_here = True
        elif token in REDIRECTS or re.fullmatch(r"\d*[<>]+&?", token):
            skip_next = True
        else:
            current.append(token)
    commands.append(current)
    heres.append(current_here)
    ops.append("")  # the last command has no following operator
    return [(w, h, o) for w, h, o in zip(commands, heres, ops) if w]


def shell_words(words):
    """WORDS with zsh's `=cmd` (the path of cmd, on any word) read as cmd."""
    return [word.lstrip("=") or word for word in words]


def long_option(arg, names):
    """The option of `names` that `arg` spells, accepting any abbreviation git would (a unique prefix of
    `--x` or longer, `--m` for --mirror), or None. An ambiguous prefix git rejects is matched too."""
    name = arg.split("=", 1)[0]
    if not arg.startswith("--") or len(name) < 3:
        return None
    return next((option for option in sorted(names) if option.startswith(name)), None)


ENV_LONG_OPTIONS = {"--unset", "--chdir", "--split-string", "--ignore-environment", "--null", "--debug",
                    "--block-signal", "--default-signal", "--ignore-signal", "--list-signal-handling", "--help",
                    "--version", "--argv0"}
ENV_LONG_VALUES = {"--unset", "--chdir", "--split-string", "--argv0"}
ENV_SHORT_VALUES = set("uCSa")


def env_settings(words):
    """Consume env's options and NAME=VALUE assignments from the front of WORDS (a list, changed in place);
    return each (option, value): `--chdir`, `--unset`, `--split-string`, or `=` for an assignment. Any unique
    prefix of a long option counts (`--c=DIR`), and `-C dir`, `-u NAME`, `-iC/x` take their value with them."""
    found = []
    while words and (words[0].startswith("-") or "=" in words[0]):
        option = words.pop(0)
        if option == "--":
            break
        if not option.startswith("-"):
            found.append(("=", option))
        elif option.startswith("--"):
            name = long_option(option, ENV_LONG_OPTIONS)
            if name in ENV_LONG_VALUES:
                found.append((name, option.split("=", 1)[1] if "=" in option else (words.pop(0) if words else "")))
        else:
            for position, letter in enumerate(option[1:], 1):
                if letter in ENV_SHORT_VALUES:
                    name = {"u": "--unset", "C": "--chdir", "S": "--split-string", "a": "--argv0"}[letter]
                    found.append((name, option[position + 1:] or (words.pop(0) if words else "")))
                    break
    return found


def env_options(words):
    """Consume env's options from the front of WORDS (a list, changed in place); return the -S/--split-string
    scripts."""
    return [value for name, value in env_settings(words) if name == "--split-string"]


def env_changes_directory(words):
    """Whether any env among WORDS runs its command in another directory (`env -C dir`, `env --c=dir`)."""
    return any(name == "--chdir" for index, word in enumerate(words) if os.path.basename(word) == "env"
               for name, _value in env_settings(list(words[index + 1:])))


def strip_prefix(words):
    """Drop VAR=value assignments and transparent wrappers in front of the real command; an env -S script's
    words take env's place."""
    words = list(words)
    while words:
        head = words[0]
        if re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*=.*", head, re.DOTALL):
            words.pop(0)
        elif os.path.basename(head) in WRAPPERS:
            words.pop(0)
        elif os.path.basename(head) == "env":
            words.pop(0)
            for script in reversed(env_options(words)):
                try:
                    words[:0] = shlex.split(script)  # `env -S 'bash -x' f` runs `bash -x f`
                except ValueError:
                    raise Unparseable("env -S script cannot be split")
        elif head in RESERVED_WORDS:
            words.pop(0)
        elif os.path.basename(head) == "timeout":
            words.pop(0)
            while words and words[0].startswith("-"):
                if words.pop(0) in ("-s", "-k", "--signal", "--kill-after") and words:
                    words.pop(0)
            del words[:1]  # the duration
        else:
            break
    return words


GIT_VALUE_OPTIONS = {"-C", "-c", "--git-dir", "--work-tree", "--namespace", "--super-prefix", "--config-env",
                     "--attr-source"}


def skip_git_options(words):
    """Drop git's global options (`git -C dir -c k=v push` runs `push`)."""
    index = 1
    while index < len(words) and words[index].startswith("-"):
        index += 2 if words[index] in GIT_VALUE_OPTIONS else 1
    return words[:1] + words[index:]


def has_output_procsub(text):
    """An output process substitution `>(cmd)` outside quotes: cmd runs, fed by data (a pipe,
    here-string or heredoc via tee) the parser cannot trace, so the gate rejects the whole line.
    Input `<(cmd)` is left to substitutions(), which parses its contents."""
    quote, index = None, 0
    while index < len(text):
        char = text[index]
        if char == "\\" and quote != "'":
            index += 2
            continue
        if quote:
            if char == quote:
                quote = None
        elif char in "'\"":
            quote = char
        elif char == ">" and text.startswith(">(", index):
            return True
        index += 1
    return False


def invocations(text, target, depth=0):
    """The arguments after `target` for every place the line runs it."""
    return [args for args, _argument in invocation_details(text, target, depth)]


def invocation_details(text, target, depth=0, seen=None):
    """(arguments, as_argument) for every place the line runs `target`; as_argument is True when it runs as
    another command's argument (find -execdir, xargs, a wrapper with options), which may run it elsewhere.
    SEEN carries what nested scripts found that the outermost call decides on (a HOME assignment)."""
    seen = {} if seen is None else seen
    if depth > 8:
        raise Refused("nesting too deep")
    if has_output_procsub(text):
        raise Unparseable("output process substitution runs a command fed by untraceable data")
    text, shell_bodies = split_heredocs(drop_quoted_cat_blocks(text))
    text = join_and_strip(text)
    nested = shell_bodies + substitutions(text)
    strings, found = [], []
    groups = [(shell_words(words), here, operator) for words, here, operator in simple_commands(text)]
    if ANSI_C in text and any(could_run(words, target) for words, _here, _operator in groups):
        raise Unparseable("ANSI-C $'...' quoting on a line that could run %s cannot be expanded by a static parser"
                          % " ".join(target))
    for index, (words, herestrings, _operator) in enumerate(groups):
        check_blocks(words)
        command = strip_prefix(words)
        seen["home"] = seen.get("home") or assigns_config_home(words)
        direct = len(words) - len(command)
        for start in range(len(words)):
            refusal = saved_config_refusal(as_git(words[start:]), start != direct) or (
                runs_git(words[start]) and override_refusal(words[start:]))
            if refusal:
                raise Refused(refusal)
        refusal = environment_refusal(words)
        if refusal:
            raise Refused(refusal)
        if command and os.path.basename(command[0]) == "gh" and command[1:2] == ["alias"] and any(
                word in ("set", "import") for word in command[2:]):
            raise Refused("gh alias set (an alias can run gh pr create or merge under another name)")
        # Any word may be a runner (sudo, xargs, find -exec, a wrapper script) whose options vary,
        # so every suffix is tried as a command: extra matches fail safe, a missed one does not.
        heads = SHELLS | {"eval", os.path.basename(target[0])}
        as_argument = bool(command) and os.path.basename(command[0]) != "eval"
        for start, word in enumerate(words):
            if os.path.basename(word) in heads or (target[0] == "git" and runs_git(word)):
                match_command(words[start:], target, found, nested, as_argument and start != direct)
                if os.path.basename(word) == "eval":
                    break  # the queued eval text already holds every later word
        nested.extend(shell_scripts(words))
        strings.extend(command_strings(words))
        for start, word in enumerate(words):
            if os.path.basename(word) == "env":
                nested.extend(env_options(words[start + 1:]))  # `env -S 'git push'` runs its split string
        if herestrings and herestring_reaches_shell(command, index, groups):
            nested.extend(herestrings)  # `bash <<< 'script'`, or `cat <<< 'script' | bash`
    for inner in nested:
        found.extend(invocation_details(inner, target, depth + 1, seen))
    for inner in dict.fromkeys(strings):
        try:
            found.extend(invocation_details(inner, target, depth + 1, seen))
        except Refused:
            raise
        except Unparseable:
            if spells_target(inner, target):
                raise  # text naming the command that the parser cannot read fails closed, as a grep would
    if depth == 0 and found and tuple(target[:2]) in CONFIG_HOME_TARGETS and seen.get("home"):
        raise Refused("the line assigns HOME, XDG_CONFIG_HOME or GIT_CONFIG_GLOBAL/SYSTEM, which points git at "
                          "config the gate cannot see")
    return found


def could_run(words, target):
    """Whether one command could run TARGET: its head is text the parser cannot read, or a word is the target's
    program, a shell, eval or a runner."""
    command = strip_prefix(words)
    return bool(command) and (ANSI_C in command[0] or any(
        os.path.basename(word) in SHELLS | RUNNERS | ARG_BUILDERS | {"eval", os.path.basename(target[0])}
        or (target[0] == "git" and runs_git(word)) for word in words))


@functools.lru_cache(maxsize=None)
def git_name(word):
    """`git` or `git-SUB` when WORD runs git: by name (`git`, /usr/lib/git-core/git-push), or a path that is a
    symlink to it under another name (/tmp/g). None otherwise. A bare name is not looked up on PATH (a stat per
    PATH entry for every word costs seconds on a WSL PATH), and a copy under another name is not seen."""
    names = [os.path.basename(word)] + ([os.path.basename(os.path.realpath(word))] if "/" in word else [])
    for name in names:
        if name == "git" or (name.startswith("git-") and len(name) > 4):
            return name
    return None


def runs_git(word):
    """Whether WORD runs git (see git_name)."""
    return git_name(word) is not None


def as_git(words):
    """WORDS as `git SUB ...` with git's global options dropped (`git-config x` runs `git config x`), or []."""
    name = git_name(words[0]) if words else None
    if not name:
        return []
    return skip_git_options(words) if name == "git" else ["git", name[4:]] + list(words[1:])


# The only keys a command line may write, through `git config`, `-c`/`--config-env` or GIT_CONFIG_KEY_<n>:
# identity, display and merge-style preferences, plus core.sshCommand when it is plain ssh (plain_ssh). Every
# other key (an alias, push or merge config, an editor, fsmonitor, credential helper or any program git runs)
# could send a later push to next or main.
WRITABLE_CONFIG = re.compile(r"user\.(?:name|email|signingkey)|color\.[a-z0-9.-]+|core\.(?:autocrlf|filemode"
                             r"|ignorecase|quotepath)|pull\.(?:rebase|ff)|merge\.conflictstyle|diff\.algorithm"
                             r"|rerere\.enabled|init\.defaultbranch|advice\.[a-z0-9-]+|commit\.gpgsign|tag\.gpgsign"
                             r"|gpg\.format|safe\.directory|fetch\.prune|column\.[a-z0-9.-]+|log\.[a-z0-9.-]+"
                             r"|format\.pretty", re.IGNORECASE)
CONFIG_READS = {"--get", "--get-all", "--get-regexp", "--get-urlmatch", "--get-color", "--get-colorbool", "--list"}
CONFIG_WRITES = {"--add", "--replace-all", "--unset", "--unset-all", "--rename-section", "--remove-section"}
CONFIG_LONG_VALUES = {"--file", "--blob", "--type", "--default", "--comment", "--value", "--url"}
CONFIG_SUBCOMMANDS = {"get": "read", "list": "read", "set": "write", "unset": "write", "rename-section": "section",
                      "remove-section": "section", "edit": "edit"}
# A line assigning (or unsetting) one of these points git at a config file (~/.gitconfig,
# $XDG_CONFIG_HOME/git/config) the gate never sees, which can hold remote.*.push or an alias.
CONFIG_HOME_NAMES = {"HOME", "XDG_CONFIG_HOME", "GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"}
# `export -n`/`declare +x` stop exporting (git then runs without it), `local` blanks it, `read`/`unset` change it.
DECLARERS = {"export", "declare", "local", "typeset", "read", "unset"}
CONFIG_HOME_TARGETS = {("git", "push"), ("git", "merge"), ("git", "rebase"), ("git", "pull")}


# ssh options that run a command (or pull in config that can): never part of a writable core.sshCommand.
SSH_COMMAND_OPTIONS = {"proxycommand", "localcommand", "permitlocalcommand", "knownhostscommand", "remotecommand",
                       "match"}


def plain_ssh(value):
    """Whether VALUE (a core.sshCommand or GIT_SSH_COMMAND) is `ssh` with only `-o Key=Value`, `-oKey=Value`,
    `-p N`, `-i PATH` and `-F PATH`, no shell metacharacter, and no -o that runs a command."""
    if value is None or re.search(r"[$`;|&()<>\n]", value):
        return False
    try:
        words = shlex.split(value)
    except ValueError:
        return False
    if words[:1] != ["ssh"]:
        return False
    index = 1
    while index < len(words):
        word, following = words[index], words[index + 1] if index + 1 < len(words) else None
        if word in ("-p", "-i", "-F", "-o"):
            if following is None or (word == "-p" and not following.isdigit()):
                return False
            option, index = following if word == "-o" else None, index + 2
        elif word.startswith("-o") and len(word) > 2:
            option, index = word[2:], index + 1
        else:
            return False
        if option is not None and ("=" not in option or option.split("=", 1)[0].lower() in SSH_COMMAND_OPTIONS):
            return False
    return True


def writable_key(key, value=None):
    """Whether a config key may be written: on the allowlist (whose character classes admit no `$`, glob,
    brace or quote, so a key the shell would still expand is never on it), or core.sshCommand set to plain
    ssh (plain_ssh) or unset (VALUE None)."""
    if key.lower() == "core.sshcommand":
        return value is None or plain_ssh(value)
    return bool(WRITABLE_CONFIG.fullmatch(key))


def assigns_config_home(words):
    """Whether one command assigns or unsets HOME, XDG_CONFIG_HOME or GIT_CONFIG_GLOBAL/SYSTEM: a `NAME=` word
    (in front of a command, after sudo or env, or alone), `export`/`declare`/`local`/`typeset`/`read`/
    `unset NAME`, or `env -u NAME`. A word that only mentions the name (`Fix HOME page`) is not one."""
    def named(word):
        return word.split("=", 1)[0] in CONFIG_HOME_NAMES

    if any("=" in word and named(word) for word in words):
        return True
    for index, word in enumerate(words):
        name = os.path.basename(word)
        if name in DECLARERS and any(named(arg) for arg in words[index + 1:] if not arg.startswith("-")):
            return True
        if name == "env" and any(option == "--unset" and value in CONFIG_HOME_NAMES
                                 for option, value in env_settings(list(words[index + 1:]))):
            return True
    return False


def config_actions(args):
    """(actions, positionals) of a `git config` call: the kinds of action it takes ('read', 'write', 'section',
    'edit'), reading any abbreviation git accepts (`--edi`, `--ge`) and short bundles (`-le`), a prefix that
    could be a write counting as one; and its operands, with each option's value consumed."""
    actions, positional, index = set(), [], 0
    while index < len(args):
        arg = args[index]
        index += 1
        if arg.startswith("--"):
            name = arg.split("=", 1)[0]
            matches = {option for option in CONFIG_READS | CONFIG_WRITES | {"--edit"}
                       if len(name) >= 3 and option.startswith(name)}
            if "--edit" in matches:
                actions.add("edit")
            if matches & {"--rename-section", "--remove-section"}:
                actions.add("section")
            if matches & CONFIG_WRITES:
                actions.add("write")
            elif matches:
                actions.add("read")
            if "=" not in arg and long_option(arg, CONFIG_LONG_VALUES) and not matches:
                index += 1
        elif arg.startswith("-") and len(arg) > 1:
            for position, letter in enumerate(arg[1:], 1):
                if letter in "ft":
                    index += 0 if arg[position + 1:] else 1
                    break
                actions.add({"e": "edit", "l": "read"}.get(letter, ""))
        else:
            positional.append(arg)
    if positional and positional[0] in CONFIG_SUBCOMMANDS:
        actions.add(CONFIG_SUBCOMMANDS[positional[0]])
    return actions - {""}, positional


def config_refusal(args, as_argument):
    """Why a `git config` call must not run, or None. Reads pass (an explicit read, or one key and nothing
    else, unless git runs as another command's argument, which xargs or find may append a value to); a write
    passes only for a literal key on WRITABLE_CONFIG; an edit or a section rename or removal never does."""
    actions, positional = config_actions(args)
    if "edit" in actions:
        return "git config --edit opens the config in an editor, which can save any key"
    if "section" in actions:
        return "git config renames or removes a whole section, which can make or move any key"
    if actions == {"read"} or (not actions and len(positional) <= 1 and not as_argument):
        return None
    operands = positional[1:] if positional[:1] in (["set"], ["unset"]) else positional
    key, value = operands[:1], (operands[1] if len(operands) > 1 else "")
    if positional[:1] == ["unset"] or any(long_option(arg, {"--unset", "--unset-all"}) for arg in args):
        value = None  # removing a key writes no value
    if not key or not writable_key(key[0], value):
        return "git config may write only the keys on WRITABLE_CONFIG in command_match.py (identity, color, " \
               "merge style) or core.sshCommand as plain ssh; `%s` is not one of them" % (key[0] if key else "")
    return None


def override_refusal(words):
    """Why a git run with `-c`/`--config-env` (KEY=VALUE for this run only) must not run, or None: the same
    allowlist as a saved write."""
    index = 1
    while index < len(words) and words[index].startswith("-"):
        word = words[index]
        value = words[index + 1] if index + 1 < len(words) else ""
        index += 2 if word in GIT_VALUE_OPTIONS else 1
        if word == "-c":
            key, _equals, setting = value.partition("=")
        elif word == "--config-env" or word.startswith("--config-env="):
            key, setting = (value if word == "--config-env" else word[len("--config-env="):]).split("=", 1)[0], ""
        else:
            continue
        if not writable_key(key, setting):
            return "git -c/--config-env sets %s for this run, a key not on WRITABLE_CONFIG in command_match.py (or " \
                   "core.sshCommand that is not plain ssh)" % key
    return None


def environment_refusal(words):
    """Why a command's GIT_CONFIG_* and GIT_SSH_COMMAND words must not run, or None: GIT_CONFIG_KEY_<n> outside
    the allowlist, GIT_CONFIG_COUNT without a literal allowlisted key for each entry, GIT_CONFIG_PARAMETERS (an
    unreadable list), GIT_CONFIG_GLOBAL/SYSTEM (a whole file the gate never sees) or a GIT_SSH_COMMAND that is
    not plain ssh (plain_ssh)."""
    keys, values, count = {}, {}, None
    for word in words:
        match = re.match(r"(GIT_CONFIG_[A-Z_0-9]*|GIT_SSH_COMMAND)=(.*)", word, re.DOTALL)
        if not match:
            continue
        name, value = match.groups()
        if name in ("GIT_CONFIG_PARAMETERS", "GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"):
            return "%s points git at config the gate cannot read" % name
        if name == "GIT_SSH_COMMAND" and not plain_ssh(value):
            return "GIT_SSH_COMMAND runs something other than plain ssh with -o/-p/-i/-F options"
        if name.startswith("GIT_CONFIG_KEY_"):
            keys[name[len("GIT_CONFIG_KEY_"):]] = value
        elif name.startswith("GIT_CONFIG_VALUE_"):
            values[name[len("GIT_CONFIG_VALUE_"):]] = value
        elif name == "GIT_CONFIG_COUNT":
            count = value
    for number, key in keys.items():
        if not writable_key(key, values.get(number, "")):
            return "GIT_CONFIG_KEY_%s=%s sets a key not on WRITABLE_CONFIG in command_match.py" % (number, key)
    if count is not None and (not count.isdigit() or any(str(n) not in keys for n in range(int(count)))):
        return "GIT_CONFIG_COUNT=%s names config keys the gate cannot read" % count
    return None


def saved_config_refusal(git, as_argument=False):
    """Why `git SUB ...` (from as_git) must not run on any line, or None: a config write off the allowlist, a
    mirror remote, or a push through plumbing."""
    if len(git) < 2:
        return None
    sub, args = git[1], git[2:]
    if sub in ("send-pack", "http-push"):
        return "git %s pushes refs without the push gates' checks" % sub
    if sub == "remote":
        operands = [arg for arg in args if not arg.startswith("-")]
        if operands[:1] == ["add"] and any(long_option(arg, {"--mirror"}) for arg in args[args.index("add") + 1:]):
            return "git remote add --mirror saves remote.<name>.mirror, which makes a later push send every branch"
    return config_refusal(args, as_argument) if sub == "config" else None


def herestring_reaches_shell(command, index, groups):
    """A here-string is a script when its command is (or a runner launches) a shell, or it pipes into one."""
    head = os.path.basename(command[0]) if command else ""
    if head in STDIN_SHELLS or (head in RUNNERS | ARG_BUILDERS and any(os.path.basename(w) in STDIN_SHELLS for w in command)):
        return True  # `bash <<< …`, `flock /tmp/l bash <<< …`, `xargs bash -c <<< …`
    while index < len(groups) and groups[index][2] in ("|", "|&"):
        raw = groups[index + 1][0] if index + 1 < len(groups) else []
        if raw and raw[0] in COMPOUND_OPENERS:  # `| if …; then bash; fi` — a shell may read the pipe inside
            return True
        following = strip_prefix(raw)
        if following and os.path.basename(following[0]) in STDIN_SHELLS:
            return True
        if following and os.path.basename(following[0]) in RUNNERS | ARG_BUILDERS and any(
                os.path.basename(w) in STDIN_SHELLS for w in following[1:]):
            return True  # `| flock … bash`, `| xargs bash -c`
        index += 1
    return False


GH_GROUP_VALUE_OPTIONS = {"-R", "--repo"}
GH_REPO_GROUPS = {"pr", "issue"}  # groups whose -R/--repo gh accepts before the subcommand
GH_ALIASES = {("pr", "new"): "create", ("pr", "ls"): "list", ("issue", "new"): "create", ("issue", "ls"): "list"}


def gh_repo_options(words, index):
    """(the -R/--repo options at WORDS[index:], the index after them)."""
    moved = []
    while index < len(words):
        word = words[index]
        if word in GH_GROUP_VALUE_OPTIONS:
            moved += words[index:index + 2]
            index += 2
        elif word.startswith("--repo=") or (word.startswith("-R") and len(word) > 2):
            moved.append(word)
            index += 1
        else:
            break
    return moved, index


def gh_subcommand_first(words):
    """`gh pr -R x new ...` and `gh -R x pr new ...` run `gh pr create ... -R x`: -R before the group or the
    subcommand moves after the subcommand (gh accepts both), and aliases resolve. A -R before any other
    command moves after it too, where that command's own parser sees it (gh api refuses it, and so does
    git_policy.api_arguments)."""
    leading, start = gh_repo_options(words, 1)
    if leading and start < len(words):
        words = words[:1] + words[start:] + leading
    if len(words) < 3 or words[1] not in GH_REPO_GROUPS:
        return words
    moved, index = [], 2
    while index < len(words):
        word = words[index]
        if word in GH_GROUP_VALUE_OPTIONS:
            moved += words[index:index + 2]
            index += 2
        elif word.startswith("--repo=") or (word.startswith("-R") and len(word) > 2):
            moved.append(word)
            index += 1
        else:
            break
    if index >= len(words):
        return words
    sub = GH_ALIASES.get((words[1], words[index]), words[index])
    return words[:2] + [sub] + words[index + 1:] + moved


SUBTREE_LONG_VALUES = {"--prefix", "--message", "--branch", "--onto", "--annotate"}
SUBTREE_SHORT_VALUES = set("Pmb")


def subtree_positionals(args):
    """The positional arguments of `git subtree ARGS`, the subcommand first. git-subtree parses with
    `git rev-parse --parseopt`, so an option may sit anywhere, a long one may be abbreviated (`--pre docs`)
    and short ones bundle: `-dP docs` and `-dPdocs` are -d with -P docs. A value letter (P, m, b) last in its
    bundle takes the next word; earlier in the bundle, the rest of the bundle. Every word that is not an
    option or its value counts, so `--` changes nothing here."""
    found, index = [], 0
    while index < len(args):
        word = args[index]
        index += 1
        if word.startswith("--"):
            index += 1 if "=" not in word and long_option(word, SUBTREE_LONG_VALUES) else 0
        elif word.startswith("-") and len(word) > 1:
            # -S takes the rest of its bundle as an optional key, so no letter after it is an option.
            value_at = next((at for at, letter in enumerate(word) if at and letter in SUBTREE_SHORT_VALUES | {"S"}),
                            None)
            index += 1 if value_at == len(word) - 1 and word[-1] != "S" else 0
        else:
            found.append(word)
    return found


def subtree_subcommand_first(words):
    """`git subtree -dP docs push origin x` as `git subtree push -- origin x`: the subcommand, wherever
    git-subtree's options left it, then the other positionals. The options themselves decide nothing here."""
    found = subtree_positionals(words[2:])
    return words[:2] + found[:1] + ["--"] + found[1:] if found else words


def match_command(words, target, found, nested, as_argument=False):
    """Record `target` (with whether it runs as another command's argument) if these words run it;
    queue eval text for parsing."""
    name = os.path.basename(words[0])
    if runs_git(words[0]):
        words, name = as_git(words), "git"
        if words[1:2] == ["subtree"]:
            words = subtree_subcommand_first(words)
    elif name == "gh":
        words = gh_subcommand_first(words)
    if name == os.path.basename(target[0]) and words[1:len(target)] == target[1:]:
        found.append((words[len(target):], as_argument))
    if name == "eval":
        nested.append(" ".join(words[1:]))


def shell_scripts(words):
    """Every word after the first -c-bearing flag or --command of a shell or runner: option values vary,
    so any may be the script."""
    first = next((i for i, word in enumerate(words) if os.path.basename(word) in SHELLS | RUNNERS), None)
    if first is None:
        return []
    for flag in range(first + 1, len(words)):
        word = words[flag]
        for long in ("--command", "--session-command"):
            if word == long:
                return list(dict.fromkeys(words[flag + 1:]))
            if word.startswith(long + "="):
                return list(dict.fromkeys([word[len(long) + 1:]] + words[flag + 1:]))
        if re.fullmatch(r"-[A-Za-z]*c[A-Za-z]*", word):
            return list(dict.fromkeys(words[flag + 1:]))
        attached = re.match(r"-[A-Za-z]*c(\S.*)", word, re.DOTALL)  # `-c'script'` shlex-joins to `-cscript`
        if attached:
            return list(dict.fromkeys([attached.group(1)] + words[flag + 1:]))
    return []


MESSAGE_OPTIONS = {"-m", "--message", "--body", "--title", "--subject"}
GH_MESSAGE_LETTERS = "tb"  # gh pr/issue create|edit|comment: -t TITLE, -b BODY


def message_positions(words):
    """Indices of WORDS that are a message value (-m TEXT, --body=TEXT) on a git commit/tag or gh pr/issue
    create/edit/comment: the only quoted text a command string may hide in that is never read as one."""
    command = strip_prefix(words)
    offset = len(words) - len(command)
    if command and runs_git(command[0]):
        named = as_git(command)
    elif command and os.path.basename(command[0]) == "gh":
        named = ["gh"] + [word for word in gh_subcommand_first(command)[1:] if not word.startswith("-")][:2]
    else:
        return set()
    if not any(named[:len(key)] == list(key) for key in BLOCK_COMMANDS):
        return set()
    letters = "m" + (GH_MESSAGE_LETTERS if named[0] == "gh" else "")
    found = set()
    for index in range(offset + 1, len(words)):
        word, previous = words[index], words[index - 1]
        if previous in MESSAGE_OPTIONS or re.fullmatch(r"-[A-Za-z]*[%s]" % letters, previous):
            found.add(index)
        elif re.match(r"(?:--message|--body|--title|--subject)=|-[%s]." % letters, word, re.DOTALL):
            found.add(index)
    return found


def command_strings(words):
    """The words of one command that may hold a command line another program runs: any word with whitespace or
    `;`/`&`/`|` in it (an ssh, watch or tmux command, `rebase --exec`, `-c core.fsmonitor=...`, an EDITOR or
    other variable's value), as written and as the value after `=` (a leading `!` dropped), and an attached
    short option's value (`-x'git push'`). Message values are data. Extra strings fail safe: a gate then reads
    a quoted mention as a run, which is what a grep would do."""
    found, skip = [], message_positions(words)
    for index, word in enumerate(words):
        if index in skip or not re.search(r"[\s;&|]", word):
            continue
        candidates = [word]
        if "=" in word:
            candidates.append(word.split("=", 1)[1])
        if re.match(r"-[A-Za-z]", word):
            head = re.match(r"\S*", word).group(0)
            candidates += [word[cut:] for cut in range(2, min(len(head), 32))]
        found += [candidate.lstrip("!") for candidate in candidates]
    return [text for text in found if text.strip()]


def spells_target(text, target):
    """Whether TEXT names TARGET as a grep would (`git push`, `git-push`, `gh pr create`), with whitespace or
    quotes between the words."""
    joiner = r"(?:-|[\s'\"\\]+)" if target[0] == "git" else r"[\s'\"\\]+"
    pattern = joiner.join(re.escape(word) for word in target)
    return bool(re.search(r"(?<![\w-])%s(?![\w-])" % pattern, text))


def invokes(text, target):
    return bool(invocations(text, target))


def absolute_literal(directory):
    """Whether DIRECTORY is an absolute path the shell takes as written: no expansion, glob or escape."""
    return directory.startswith("/") and not re.search(r"[$`*?\[\\\x01]", directory)


def cd_target(text, target):
    """DIR when the line is `cd DIR && TARGET ...` with an absolute literal DIR and runs TARGET nowhere else; else None."""
    if len(invocations(text, target)) != 1:
        return None
    groups = simple_commands(join_and_strip(split_heredocs(drop_quoted_cat_blocks(text))[0]))
    if len(groups) < 2:
        return None
    (first, _, operator), (second, _, _) = groups[0], groups[1]
    if any(re.match(r"GIT_(?:DIR|WORK_TREE)=", word) for word in second):
        return None  # gh reads the repository GIT_DIR names, not DIR's
    stripped = strip_prefix(second)
    if env_changes_directory(second[:len(second) - len(stripped)]):
        return None  # `env -C/x` runs it in /x, not DIR
    second = stripped
    if second[:1] == ["gh"]:
        second = gh_subcommand_first(second)
    if first[:1] != ["cd"] or len(first) != 2 or operator != "&&" or second[:len(target)] != target:
        return None
    return first[1] if absolute_literal(first[1]) else None


def main():
    target = sys.argv[1:]
    want_cd = target[:1] == ["--cd-target"]
    target = target[1:] if want_cd else target
    if not target:
        print("usage: command_match.py [--cd-target] WORD...", file=sys.stderr)
        return 2
    try:
        text = sys.stdin.read()
        if want_cd:
            print(cd_target(text, target) or "")
            return 0
        return 0 if invokes(text, target) else 1
    except Unparseable as error:
        print("command_match: %s" % error, file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
