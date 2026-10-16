"""Does a tool call write a review or tests marker by hand? Used by marker-write-gate.sh, and by
marker_ledger.py to tell which tool calls may stamp one.

  printf '%s' "$PAYLOAD" | python3 -I marker_write.py

Prints a reason and exits 0 to block, prints nothing to allow; any other exit makes the
hook block. The tests, review-shipping and code-review markers are proof that a suite or a
review ran on a commit, so only bin/run-tests.sh and bin/review-marker.sh may write them.
attest/ is not covered: the review lenses write their attestations there.

This is the cheap first line: it refuses the spellings it can read before a tool runs.
Write, Edit, MultiEdit and NotebookEdit are refused for a path in one of the three
directories, the manifest beside them or the marker ledger (resolving symlinks, any letter
case). A Bash line naming one of those paths literally passes only when every command on it
is a read, one of the two scripts (from the checkout, on a line with no cd) or exactly
`just test`; a write redirect, substitution or heredoc fails closed. So does a line that
runs `marker_manifest.py stamp` itself, which only the two scripts call. A path the shell
builds (a variable, glob, brace or ANSI-C word) and a tree copied in by tar, cp -r and the
like are not read here: marker_ledger.py checks the outcome after every call, and the PR
gates refuse a marker without its manifest line. Python 3.8-compatible.
"""

import functools
import json
import os
import re
import shlex
import subprocess
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from command_match import (  # noqa: E402
    Unparseable, absolute_literal, drop_quoted_cat_blocks, join_and_strip, simple_commands, split_heredocs,
    split_operators, strip_prefix, substitutions)

LEDGER_DIR = "ois-marker-ledger"
PROTECTED = re.compile(r"(?:^|/)\.claude/markers/(?:(?:tests|review-shipping|code-review)(?:/|$)|manifest$)"
                       r"|(?:^|/)%s(?:/|$)" % LEDGER_DIR, re.IGNORECASE)
MARKERS_DIR = re.compile(r"(?:^|/)\.claude/markers/?$", re.IGNORECASE)
NAMES = "names a marker, the manifest or the marker ledger"
EDIT_TOOLS = {"Write", "Edit", "MultiEdit", "NotebookEdit"}
READS = {"ls", "cat", "test", "[", "find", "stat", "wc", "head", "tail", "grep", "echo", "printf", "cd", "pwd",
         "true"}
STAMPING_SCRIPTS = {"run-tests.sh", "review-marker.sh"}
FIND_WRITES = {"-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf", "-fls"}
SEPARATORS = {";", "&", "&&", "|", "||", "|&", "(", ")", "\n"}
WRITE_REDIRECTS = {">", ">>", ">|", "&>", "&>>", "<>"}
DIRECTORY_CHANGES = {"cd", "pushd", "popd"}
REVIEW_KINDS = {"code-review": ("code-review",), "shipping": ("code-review", "review-shipping")}
# What a read beside a stamper may hold of the shell's own: the last exit status, and nothing else it expands.
EXIT_STATUS = re.compile(r"\$\?|\$\{PIPESTATUS\[\d\]\}")
EXPANDS = re.compile(r"[$`]")
# A redirect target the shell rewrites (an expansion, glob, brace, tilde, escape, zsh `=cmd` or `>!`).
TARGET_EXPANDS = re.compile(r"[$`*?\[\]{}~!\\\x01]|^=")


def protected(path, cwd):
    """Whether PATH, as given or with symlinks resolved, is a marker, the manifest or in the ledger, or is
    the markers directory itself."""
    absolute = os.path.normpath(os.path.join(cwd, os.path.expanduser(path)))
    return any(pattern.search(candidate) for pattern in (PROTECTED, MARKERS_DIR)
               for candidate in (absolute, os.path.realpath(absolute)))


def mentions(command, cwd):
    """Whether the line names a protected path literally: as a word, a redirect target, or a path inside a
    quoted script. A commit or PR body in a quoted cat block, and a heredoc no shell reads, are data."""
    try:
        code, shell_bodies = split_heredocs(drop_quoted_cat_blocks(command))
    except Unparseable:
        return bool(PROTECTED.search(command) or re.search(r"\.claude/markers(?:/|\s|$)", command, re.I))
    for part in [code] + shell_bodies:
        try:
            words = shlex.split(part, posix=True)
        except ValueError:
            words = part.split()
        tokens = [token for word in words for token in [word] + re.split(r"[\s'\"(),;<>|&=]+", word)]
        if any(protected(token.lstrip("<>&|;("), cwd) for token in tokens if "/" in token):
            return True
    return False


def tokens_of(text):
    """TEXT as shell words and operators, redirects kept."""
    lexer = shlex.shlex(text, posix=True, punctuation_chars="();<>|&\n")
    lexer.whitespace = " \t\r"
    lexer.whitespace_split = True
    lexer.commenters = ""
    try:
        return [part for token in lexer for part in split_operators(token)]
    except ValueError as error:
        raise Unparseable(str(error))


def write_targets(text):
    """The target of every write redirect in TEXT (`> f`, `>> f`, `&> f`, `>& f`, `<> f`); `>&2` names none.
    A redirect with no target cannot be read."""
    tokens, found = tokens_of(text), []
    for index, token in enumerate(tokens):
        target = tokens[index + 1] if index + 1 < len(tokens) else ""
        if token in WRITE_REDIRECTS or (token == ">&" and not re.fullmatch(r"\d+|-", target)):
            if not target or target in SEPARATORS or target in WRITE_REDIRECTS:
                raise Unparseable("a write redirect to nothing")
            found.append(target)
    return found


def outside_guarded(target, cwd):
    """Whether a redirect target is a path the shell takes as written that lies, as given and with symlinks
    resolved, outside every `.claude/` and outside the git directory (and so the ledger). Inside the checkout
    it is judged by its path from the checkout root: an issue worktree sits under the main checkout's
    `.claude/worktrees/`, and a log written in one is not a write to `.claude/`. git prints the root with
    symlinks resolved, so the root as CWD spells it (/var/... for /private/var/... on macOS) counts too."""
    if TARGET_EXPANDS.search(target):
        return False
    absolute = os.path.normpath(os.path.join(cwd, target))
    top = git_line(cwd, "rev-parse", "--show-toplevel")
    logical = os.path.normpath(os.path.join(cwd, os.path.relpath(top, os.path.realpath(cwd)))) if top else None
    tops = [found.rstrip(os.sep) + os.sep for found in (top, os.path.realpath(top) if top else None, logical)
            if found]
    git_dirs = [os.path.realpath(found) for found in (
        git_line(cwd, "rev-parse", "--absolute-git-dir"),
        git_line(cwd, "rev-parse", "--path-format=absolute", "--git-common-dir")) if found]
    for candidate in (absolute, os.path.realpath(absolute)):
        inside = next((found for found in tops if candidate.startswith(found)), "")
        parts = candidate[len(inside):].lower().split(os.sep)
        if ".claude" in parts or ".git" in parts or LEDGER_DIR in parts:
            return False
        if any(candidate == found or candidate.startswith(found.rstrip(os.sep) + os.sep) for found in git_dirs):
            return False
    return True


def plain_read(words):
    """Whether one command is a read by its bare name (not a path, which could be any program) that expands
    nothing but the last exit status, so it can only print."""
    if words[0] not in READS - DIRECTORY_CHANGES or (words[0] == "find" and any(word in FIND_WRITES for word in words)):
        return False
    return not any(EXPANDS.search(EXIT_STATUS.sub("", word)) for word in words[1:])


def stamps_manifest(command):
    """Whether a Bash line runs `marker_manifest.py stamp` itself; only the two stamping scripts call it."""
    try:
        code, bodies = split_heredocs(drop_quoted_cat_blocks(command))
        groups = [words for part in [code] + bodies for words, _here, _operator in simple_commands(join_and_strip(part))]
    except Unparseable:
        return "marker_manifest" in command and "stamp" in command
    return any(os.path.basename(word) == "marker_manifest.py" and words[index + 1:index + 2] == ["stamp"]
               for words in groups for index, word in enumerate(words))


def commands(command):
    """Split the line into argv lists, refusing what a static read cannot follow."""
    if re.search(r"\$\(|`|<\(|>\(|<<", command):
        raise Unparseable("a substitution or heredoc")
    tokens = tokens_of(command)
    found, current, index = [], [], 0
    while index < len(tokens):
        token = tokens[index]
        target = tokens[index + 1] if index + 1 < len(tokens) else ""
        if token in SEPARATORS:
            found.append(current)
            current = []
        elif token in WRITE_REDIRECTS:
            if target != "/dev/null":
                raise Unparseable("a write redirect to %s" % (target or "nothing"))
            index += 1
        elif token in (">&", "<&"):
            if token == ">&" and not re.fullmatch(r"\d+|-", target):
                raise Unparseable("a write redirect to %s" % (target or "nothing"))
            index += 1
        elif token == "<":
            index += 1
        else:
            current.append(token)
        index += 1
    found.append(current)
    return [words for words in found if words]


@functools.lru_cache(maxsize=None)
def toplevel(cwd):
    return git_line(cwd, "rev-parse", "--show-toplevel")


def git_line(cwd, *args):
    """One line of git output in CWD, or None."""
    try:
        result = subprocess.run(["git"] + list(args), cwd=cwd, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, universal_newlines=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def stamping_script(path, cwd):
    """Whether PATH is this checkout's own bin/run-tests.sh or bin/review-marker.sh."""
    name = os.path.basename(path)
    root = toplevel(cwd) if name in STAMPING_SCRIPTS else None
    return bool(root) and os.path.realpath(os.path.join(cwd, path)) == os.path.realpath(os.path.join(root, "bin", name))


def allowed(words, cwd, moves):
    command = strip_prefix(words)
    if not command:
        return True
    if os.path.basename(command[0]) in ("bash", "sh") and len(command) > 1:
        command = command[1:]
    name = os.path.basename(command[0])
    if name in STAMPING_SCRIPTS:
        return not moves and stamping_script(command[0], cwd)
    if name == "just" and command[1:] == ["test"]:
        return not moves
    if name == "find":
        return not any(word in FIND_WRITES for word in command[1:])
    return name in READS


def stamps(words, cwd):
    """The (kind, sha) markers one command may stamp: `just test` or bin/run-tests.sh with no arguments
    stamps tests for HEAD, bin/review-marker.sh KIND SHA its review markers. None when it is no stamper."""
    if words[:1] in (["bash"], ["sh"]):
        words = words[1:]
    if words == ["just", "test"] or (len(words) == 1 and stamping_script(words[0], cwd)
                                         and os.path.basename(words[0]) == "run-tests.sh"):
        head = git_line(cwd, "rev-parse", "HEAD")
        return [("tests", head)] if head else []
    if len(words) == 3 and os.path.basename(words[0]) == "review-marker.sh" and stamping_script(words[0], cwd) \
            and words[1] in REVIEW_KINDS and re.fullmatch(r"[0-9a-f]{40}", words[2]):
        return [(kind, words[2]) for kind in REVIEW_KINDS[words[1]]]
    return None


def cd_stamper(groups):
    """The checkout root DIR when the line is exactly `cd DIR && just test`, DIR an absolute literal path
    that is itself a checkout's root (the form a shared working directory needs); else None."""
    if len(groups) != 2:
        return None
    (first, _, operator), (second, _, _) = groups
    if first[:1] != ["cd"] or len(first) != 2 or operator != "&&" or second != ["just", "test"]:
        return None
    directory = first[1]
    root = toplevel(directory) if absolute_literal(directory) and os.path.isdir(directory) else None
    return root if root and os.path.realpath(root) == os.path.realpath(directory) else None


def stampers(command, cwd):
    """(root, kind, sha) for every marker a Bash line may stamp, or [] unless the line is nothing but
    stampers and reads, so nothing on it can ride on the stamper's authority: no substitution; every
    command a stamper as stamps reads it or a plain_read, which a prefix (`X=1`, `timeout 5`) or a shell
    reading a heredoc is not; every write redirect to a literal path
    outside `.claude/` and the git directory (outside_guarded: `just test > /tmp/out.txt 2>&1`); and
    no cd but the one form cd_stamper reads, which stamps tests for that checkout's HEAD."""
    try:
        text = join_and_strip(split_heredocs(drop_quoted_cat_blocks(command))[0])
        if substitutions(text):
            return []
        commands_found = simple_commands(text)
        targets = write_targets(text)
    except Unparseable:
        return []
    elsewhere = cd_stamper(commands_found)
    root = elsewhere or toplevel(cwd)
    if not root or not all(outside_guarded(target, elsewhere or cwd) for target in targets):
        return []
    if elsewhere:
        head = git_line(elsewhere, "rev-parse", "HEAD")
        return [(elsewhere, "tests", head)] if head else []
    found = []
    for words, _here, _operator in commands_found:
        stamped = stamps(words, cwd)
        if stamped is not None:
            found += [(root, kind, sha) for kind, sha in stamped]
        elif not plain_read(words):
            return []
    return found


def bash_reason(command, cwd):
    if stamps_manifest(command):
        return "the line runs marker_manifest.py stamp itself"
    if not mentions(command, cwd):
        return None
    try:
        lines = commands(command)
    except Unparseable as error:
        return "the line %s and has %s" % (NAMES, error)
    moves = any(os.path.basename(strip_prefix(words)[0]) in DIRECTORY_CHANGES for words in lines if strip_prefix(words))
    for words in lines:
        if not allowed(words, cwd, moves):
            return "the line %s and runs `%s`" % (NAMES, " ".join(words[:3]))
    return None


def reason(payload, cwd):
    tool = payload.get("tool_name")
    tool_input = payload.get("tool_input") or {}
    if tool in EDIT_TOOLS:
        for key in ("file_path", "notebook_path"):
            path = tool_input.get(key)
            if isinstance(path, str) and path and protected(path, cwd):
                return "%s would write %s" % (tool, path)
        return None
    if tool == "Bash":
        return bash_reason(str(tool_input.get("command") or ""), cwd)
    return None


def payload_cwd(payload):
    cwd = payload.get("cwd")
    return cwd if isinstance(cwd, str) and os.path.isdir(cwd) else os.getcwd()


def main():
    payload = json.loads(sys.stdin.read())
    if not isinstance(payload, dict):
        raise ValueError("the payload is not a JSON object")
    found = reason(payload, payload_cwd(payload))
    if found:
        print(found)
    return 0


if __name__ == "__main__":
    sys.exit(main())
