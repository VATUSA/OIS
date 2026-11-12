"""Push and merge policies for the git gates, decided on the parsed command line.

  printf '%s' "$COMMAND" | python3 git_policy.py protected-push CURRENT_BRANCH
  printf '%s' "$COMMAND" | python3 git_policy.py non-main-integration
  printf '%s' "$COMMAND" | python3 git_policy.py update-branch
  printf '%s' "$COMMAND" | python3 git_policy.py pr-head
  printf '%s' "$COMMAND" | python3 git_policy.py api-write
  printf '%s' "$COMMAND" | python3 git_policy.py push-checkout
  printf '%s' "$COMMAND" | python3 git_policy.py push-destinations
  printf '%s' "$COMMAND" | python3 git_policy.py primary-branch
  printf '%s' "$COMMAND" | python3 git_policy.py stage-all
  printf '%s' "$COMMAND" | python3 git_policy.py migrations

Exit 0 with nothing printed: allow. Exit 0 with a reason printed: block. Any other exit
(unparseable line, python3 missing): the caller blocks. `update-branch` instead prints
one PR per line (`OWNER/REPO#N`, `#N` for the current repo, or `?` where the line does not
name one it can be checked by); the caller asks GitHub for each PR's base. `pr-head` prints
nothing when the line opens no PR, `=BRANCH` for the branch the PR opens from (`=` alone for
the current branch), or `?REASON` when that cannot be checked. `api-write` blocks every gh api
write but the allowlisted ones, and every GraphQL call but a literal read. `push-checkout` prints
where the push runs (see push_checkout), `push-destinations` each branch a push names (see
push_destinations). `primary-branch` blocks creating, renaming or switching a branch in the primary
checkout (gh pr checkout included), which stays on next; `stage-all` blocks `git add -A`, `.`, `*`, `--all` and a
bare -u, and `git commit -a`; `migrations` lists the commits and the added paths for migration-gate.sh (see
staged_migrations). Integration, pr-head and primary-branch read the repository in the current directory.
Python 3.8-compatible.
"""

import os
import re
import subprocess
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from command_match import (  # noqa: E402
    ANSI_C, Unparseable, absolute_literal, as_git, cd_target, drop_quoted_cat_blocks, env_changes_directory,
    gh_subcommand_first, invocation_details, invocations, join_and_strip, long_option, runs_git, shell_words,
    simple_commands, split_heredocs, strip_prefix)

# Never pushed to and never merged into by an agent: work reaches them only through a reviewed PR.
PROTECTED = {"main", "next", "master"}
# What a branch may merge or pull from: next (the integration branch every PR targets), and main for a release
# back-merge. A rebase (git rebase, --onto, git pull --rebase) replays the branch onto its source, and a branch
# rebased onto main would carry main's history into a PR against next, so a rebase takes next alone.
INTEGRATION_SOURCES = {"next", "main"}
REBASE_SOURCES = {"next"}
# `@` is git's shorthand for HEAD.
CURRENT_HEAD = {"HEAD", "@"}
# A destination the shell or git would expand (a variable, substitution, brace, glob) cannot be checked.
UNRESOLVED_REF = re.compile(r"[$`{}*?\[\\~]")
# A git repository option (`-C dir`, also env's `-C`/`-iC/x`), or GIT_DIR / GIT_WORK_TREE: git runs somewhere
# other than the hook's cwd. env's --chdir, under any abbreviation, is read by env_changes_directory.
REPOSITORY_OPTION = re.compile(r"(?:^|\s)(?:-[A-Za-z]*C\S*|--git-dir|--work-tree)(?:[\s=]|$)|\bGIT_(?:DIR|WORK_TREE)=")
PUSH_VALUE_OPTIONS = {"--repo", "-o", "--push-option", "--receive-pack", "--exec"}
PUSH_EVERY_BRANCH = {"--all", "--mirror", "--branches"}
MERGE_VALUE_OPTIONS = {"-m", "-F", "--file", "-s", "--strategy", "-X", "--strategy-option", "--into-name"}
REBASE_VALUE_OPTIONS = {"-x", "--exec", "-s", "--strategy", "-X", "--strategy-option"}
PULL_VALUE_OPTIONS = {"-s", "--strategy", "-X", "--strategy-option", "--depth", "--upload-pack", "-o",
                      "--server-option", "--negotiation-tip"}
# gh api's flags (gh 2.x): the short ones taking a value, the short booleans, and the long ones taking a value.
API_SHORT_VALUES = set("XHfFpqt")
API_SHORT_FLAGS = set("i")
API_LONG_VALUES = {"--method", "--header", "--raw-field", "--field", "--input", "--jq", "--template", "--hostname",
                   "--preview", "--cache"}
PR_VALUE_OPTIONS = {"-R", "--repo"}
UPDATE_BRANCH_ENDPOINT = re.compile(r"^/?(?:repos/([^/\s]+/[^/\s]+)/)?pulls/(\d+)/update-branch/?(?:\?.*)?$")
PR_URL = re.compile(r"^https://github\.com/([^/\s]+/[^/\s]+)/pull/(\d+)(?:[/?#].*)?$")
# `--abort`, `--continue` and friends finish or drop an integration already under way.
INTEGRATION_CONTROL = {"--abort", "--continue", "--quit", "--skip", "--edit-todo", "--show-current-patch"}
# gh pr create's value-taking options (gh 2.x); every other flag it accepts is a boolean.
PR_CREATE_SHORT_VALUES = set("aBbFHlmprTtR")
PR_CREATE_SHORT_FLAGS = set("defhw")
PR_CREATE_LONG_VALUES = {"--assignee", "--base", "--body", "--body-file", "--head", "--label", "--milestone",
                         "--project", "--reviewer", "--template", "--title", "--repo", "--recover"}
API_PULLS = re.compile(r"^(?:(?:repos/[^/\s]+/[^/\s]+|repositories/\d+)/)?pulls$")
# The only gh api writes the workflow makes; update-branch-gate.sh checks that PR's base separately. An issue
# comment through REST is the fallback .claude/rules/github-issues.md gives when GraphQL (which gh issue comment
# uses) is throttled, and editing one (PATCH) is how ticket-worker keeps its plan comment current; attribution-gate.sh
# checks both bodies.
API_WRITE_ALLOWLIST = [("PUT", re.compile(r"^repos/[^/\s]+/[^/\s]+/pulls/\d+/update-branch$")),
                       ("POST", re.compile(r"^repos/[^/\s]+/[^/\s]+/issues/\d+/comments$")),
                       ("PATCH", re.compile(r"^repos/[^/\s]+/[^/\s]+/issues/comments/\d+$"))]
API_HOST_PREFIX = re.compile(r"^(?:https?://[^/\s]+(?:/api/v3)?)?/*")


def positionals(args, value_options):
    """Positional arguments, with each option's separate value consumed and --opt=value kept whole.

    For a push every positional is checked, the remote included, so `--repo X` cannot shift a
    refspec into the remote's slot; consuming values only avoids refusing `-o main`.
    """
    found, index = [], 0
    while index < len(args):
        arg = args[index]
        if arg == "--":
            found.extend(args[index + 1:])
            break
        if arg.startswith("-"):
            index += 2 if arg in value_options else 1
            continue
        found.append(arg)
        index += 1
    return found


def branch_name(ref):
    ref = ref.lstrip("+")
    # git resolves a destination of `heads/main` to refs/heads/main.
    for prefix in ("refs/heads/", "heads/", "refs/remotes/origin/", "origin/"):
        if ref.startswith(prefix):
            return ref[len(prefix):]
    return ref


def moves_repository(text):
    """Whether the line runs git somewhere other than the hook's cwd: a cd, pushd or popd it runs
    (parsed, so a commit message saying `cd` is data), or a repository option outside heredoc data."""
    if any(invocations(text, [word]) for word in ("cd", "pushd", "popd")):
        return True
    if any(arg in ("-execdir", "-okdir") for args in invocations(text, ["find"]) for arg in args):
        return True  # find runs the command from each match's directory
    if any(env_changes_directory(["env"] + args) for args in invocations(text, ["env"])):
        return True
    code, shell_bodies = split_heredocs(drop_quoted_cat_blocks(text))
    return any(REPOSITORY_OPTION.search(part) for part in [code] + shell_bodies)


def protected_push(text, current):
    """A reason the line's push could write next or main, or None.

    CURRENT is the hook cwd's branch, which says nothing once the line moves directory or repository,
    so then a push that leaves the branch to git (no refspec, or HEAD/@) must name it instead.
    """
    pushes = invocation_details(text, ["git", "push"])
    moved = bool(pushes) and moves_repository(text)
    for args, as_argument in pushes:
        if any(long_option(arg, PUSH_EVERY_BRANCH) for arg in args):
            return "pushing every branch includes next and main"
        # A push run as another command's argument (find -execdir, xargs) may run in any checkout.
        unknown = ("this push runs as another command's argument" if as_argument and not moved else
                   "this line changes directory or repository") + \
            ", so the branch it pushes is unknown; name it: git push origin BRANCH"
        here_unknown = moved or as_argument
        words = positionals(args, PUSH_VALUE_OPTIONS)
        if len(words) <= 1 and here_unknown:
            return unknown
        if len(words) <= 1 and current in PROTECTED:
            return "you are on %s and this pushes the current branch" % current
        for refspec in words:
            reason = refspec_reason(refspec, current, here_unknown)
            if reason:
                return reason if reason != "unknown" else unknown
    return subtree_push_reason(text)


def subtree_push_reason(text):
    """A reason `git subtree push --prefix=DIR REMOTE REF` could write next or main, or None: it pushes the split
    history to REF on REMOTE, so REF is checked as a push destination."""
    for args in invocations(text, ["git", "subtree", "push"]):
        words = positionals(args, set())  # command_match.subtree_subcommand_first leaves `-- REMOTE REF`
        if len(words) != 2:
            return "git subtree push names no single remote and branch the gate can check"
        reason = refspec_reason(words[1], "", False) or refspec_reason(words[0], "", False)
        if reason:
            return reason if reason != "unknown" else "git subtree push names a destination the gate cannot check"
    return None


def refspec_reason(refspec, current, moved):
    """Why one push refspec (or the remote, checked the same way) could write next or main, `unknown`, or None."""
    if refspec.lstrip("+") in ("", ":"):
        return "the refspec `%s` pushes every matching branch, next and main included" % refspec
    source, colon, destination = refspec.lstrip("+").rpartition(":")
    if UNRESOLVED_REF.search(destination):
        return "the refspec `%s` names a destination the shell or git expands, so it cannot be checked" % refspec
    if colon and not destination:
        return "the refspec `%s` has an empty destination" % refspec
    if branch_name(destination) in PROTECTED:
        return "the refspec `%s` writes %s" % (refspec, branch_name(destination))
    head = destination if not colon else source
    if head in CURRENT_HEAD and moved:
        return "unknown"
    if not colon and destination in CURRENT_HEAD and current in PROTECTED:
        return "you are on %s and `%s` pushes it" % (current, destination)
    return None


def canonical_onto(arg):
    """`--ont=x` or `--o x` as git reads it: --onto, since git accepts any unique prefix of a long option."""
    if long_option(arg, {"--onto"}):
        _name, equals, value = arg.partition("=")
        return "--onto" + equals + value
    return arg


def allowed_integration(ref, sources=INTEGRATION_SOURCES):
    return branch_name(ref) in sources


def git_output(*args):
    """A git command's stripped stdout in the current directory, or None when it fails."""
    try:
        result = subprocess.run(["git"] + list(args), stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                universal_newlines=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return None
    return result.stdout.strip() if result.returncode == 0 else None


def implicit_source(text, sources=INTEGRATION_SOURCES):
    """A merge, rebase or pull naming no source takes the current branch's upstream, which may not be in SOURCES."""
    if moves_repository(text):
        return "the upstream of whichever branch this line ends up in (it changes directory or repository)"
    upstream = git_output("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}")
    if not upstream:
        return "the current branch's upstream, which git cannot resolve"
    return None if allowed_integration(upstream, sources) else "%s (the current branch's upstream)" % upstream


FALSE_WORDS = ("false", "no", "off", "0")
# Every spelling of pull.rebase or branch.<name>.rebase a command line can carry: `-c pull.rebase[=v]`,
# `--config-env=pull.rebase=VAR`, GIT_CONFIG_KEY_<n>=pull.rebase, or a `git config` write on the same line.
REBASE_KEY = re.compile(r"(?:pull|branch\.\S*)\.rebase", re.IGNORECASE)
REBASE_OVERRIDE = re.compile(r"^(?:pull|branch\.[^=\s]*)\.rebase(?:=(.*))?$", re.IGNORECASE | re.DOTALL)


def line_rebase_config(text):
    """What the line itself says about pull.rebase / branch.<name>.rebase: None when it never names either,
    False when every mention is a `-c KEY=<false>` override, else True (a true value, a key with no value,
    which git reads as true, or a mention the gate cannot evaluate, such as a `git config` write)."""
    code = split_heredocs(drop_quoted_cat_blocks(text))[0]
    if not REBASE_KEY.search(code):
        return None
    mentions, values = 0, []
    for raw, _here, _operator in simple_commands(join_and_strip(code)):
        words = shell_words(raw)
        mentions += sum(len(REBASE_KEY.findall(word)) for word in words)
        for index, word in enumerate(words[:-1]):
            if word == "-c":
                match = REBASE_OVERRIDE.match(words[index + 1])
                if match:
                    values.append(match.group(1))
    if mentions == 0 or len(values) != mentions:
        return True
    return any(value is None or value.lower() not in FALSE_WORDS for value in values)


def pull_rebases(args, text):
    """Whether `git pull ARGS` rebases: the last of -r/--rebase[=VALUE]/--no-rebase decides; else what the line
    sets for pull.rebase or branch.<name>.rebase (line_rebase_config); else the saved config. A line that runs
    git elsewhere has config the gate cannot read, so it counts as a rebase (the stricter)."""
    mode = None
    for arg in args:
        if arg == "--":
            break
        if long_option(arg, {"--no-rebase"}):
            mode = False
        elif long_option(arg, {"--rebase"}):
            _name, equals, value = arg.partition("=")
            mode = not equals or value.lower() not in FALSE_WORDS
        elif re.fullmatch(r"-[A-Za-z]+", arg) and "r" in arg:
            mode = True
    if mode is not None:
        return mode
    on_line = line_rebase_config(text)
    if on_line is not None:
        return on_line
    if moves_repository(text):
        return True
    branch = git_output("symbolic-ref", "--quiet", "--short", "HEAD")
    for key in (["branch.%s.rebase" % branch] if branch else []) + ["pull.rebase"]:
        configured = git_output("config", "--get", key)
        if configured is not None:
            return configured.lower() not in ("",) + FALSE_WORDS
    return False


def non_main_integration(text):
    """The first merge, rebase or pull source that is not allowed, or None: a merge or pull may take next or main, a
    rebase (git rebase, its --onto, git pull --rebase) next alone. `git subtree` pull, merge and add count
    (subtree_source).

    No source means the upstream, and `-` means the previously checked-out branch.
    """
    implicit, implicit_rebase = False, False
    for args in invocations(text, ["git", "merge"]):
        if "-" in args:
            return "- (the previously checked-out branch)"
        refs = positionals(args, MERGE_VALUE_OPTIONS)
        for ref in refs:
            if not allowed_integration(ref):
                return ref
        implicit = implicit or not (refs or set(args) & INTEGRATION_CONTROL)
    for args in invocations(text, ["git", "rebase"]):
        args = [canonical_onto(arg) for arg in args]
        if "-" in args:
            return "- (the previously checked-out branch)"
        onto_given = False
        for index, arg in enumerate(args):
            onto = args[index + 1] if arg == "--onto" and index + 1 < len(args) else None
            if arg.startswith("--onto="):
                onto = arg.split("=", 1)[1]
            if onto is not None:
                onto_given = True
                if not allowed_integration(onto, REBASE_SOURCES):
                    return "%s (a rebase takes next only)" % onto
        upstream = positionals(args, REBASE_VALUE_OPTIONS | {"--onto"})[:1]
        if upstream and not allowed_integration(upstream[0], REBASE_SOURCES):
            return "%s (a rebase takes next only)" % upstream[0]
        # `--onto next` with no upstream replays only this branch's own commits onto next.
        implicit_rebase = implicit_rebase or not (upstream or onto_given or
                                                  set(args) & (INTEGRATION_CONTROL | {"--root"}))
    for args in invocations(text, ["git", "pull"]):
        sources = REBASE_SOURCES if pull_rebases(args, text) else INTEGRATION_SOURCES
        words = positionals(args, PULL_VALUE_OPTIONS)
        for refspec in words[1:]:
            if not allowed_integration(refspec.split(":")[0], sources):
                return refspec if sources is INTEGRATION_SOURCES else "%s (a rebase takes next only)" % refspec
        if len(words) <= 1:
            if sources is REBASE_SOURCES:
                implicit_rebase = True
            else:
                implicit = True
    return subtree_source(text) or (implicit_source(text) if implicit else None) or \
        (implicit_source(text, REBASE_SOURCES) if implicit_rebase else None)


def subtree_source(text):
    """The first `git subtree pull|merge|add` source that is not next or main, or None. Each merges the history of the
    ref it names last (`pull REPOSITORY REF`, `merge COMMIT`, `add [REPOSITORY] REF`) under the prefix."""
    for sub, counts in (("pull", (2,)), ("merge", (1,)), ("add", (1, 2))):
        for args in invocations(text, ["git", "subtree", sub]):
            words = positionals(args, set())
            if len(words) not in counts:
                return "git subtree %s (it names no single ref the gate can check)" % sub
            if not allowed_integration(words[-1]):
                return words[-1]
    return None


def option_value(args, names):
    """The value of the last of `names` given as `-R x`, `--repo x` or `--repo=x`, or None."""
    value = None
    for index, arg in enumerate(args):
        if arg in names and index + 1 < len(args):
            value = args[index + 1]
        elif "=" in arg and arg.split("=", 1)[0] in names:
            value = arg.split("=", 1)[1]
    return value


def update_branch_targets(text):
    """Every PR the line would bring up to date with its base, as `OWNER/REPO#N`, `#N` or `?`.

    GitHub merges the PR's own base, so the gate must see which PR; `?` (a variable, a branch
    name, a rebase, a GraphQL mutation) cannot be checked and is refused.
    """
    found = []
    # `#N` is looked up in the hook's cwd; a cd, -C, GIT_DIR or GH_REPO points gh at another repository.
    elsewhere = moves_repository(text) or bool(re.search(r"\bGH_REPO=", text))
    for endpoint, _method, args in api_calls(text):
        if api_endpoint(endpoint) == "graphql":
            if any("updatePullRequestBranch" in arg for arg in args):
                found.append("?")
            continue
        if "update-branch" not in endpoint:
            continue
        match = UPDATE_BRANCH_ENDPOINT.match(endpoint)
        found.append(("%s#%s" % (match.group(1) or "", match.group(2))) if match else "?")
    for args in invocations(text, ["gh", "pr", "update-branch"]):
        words = positionals(args, PR_VALUE_OPTIONS)
        repo = option_value(args, PR_VALUE_OPTIONS) or ""
        if any(arg == "--rebase" for arg in args) or len(words) != 1:
            found.append("?")
            continue
        url = PR_URL.match(words[0])
        if url:
            found.append("%s#%s" % (url.group(1), url.group(2)))
        elif words[0].isdigit():
            found.append("%s#%s" % (repo, words[0]))
        else:
            found.append("?")
    return ["?" if target.startswith("#") and elsewhere else target for target in found]


def pr_create_options(args):
    """(head, repo) from gh pr create's arguments, or raise ValueError on a short flag it does not know.

    pflag accepts `-Hx`, `-dH x` and `--head=x`, so short bundles are walked letter by letter.
    """
    found, index = {}, 0
    while index < len(args):
        arg = args[index]
        index += 1
        if arg == "--":
            break
        if arg.startswith("--"):
            name, equals, value = arg.partition("=")
            if name in PR_CREATE_LONG_VALUES and not equals:
                value = args[index] if index < len(args) else ""
                index += 1
            if name in ("--head", "--repo"):
                found[name] = value
        elif arg.startswith("-") and len(arg) > 1:
            for position, letter in enumerate(arg[1:], 1):
                if letter in PR_CREATE_SHORT_VALUES:
                    value = arg[position + 1:]
                    if not value:
                        value = args[index] if index < len(args) else ""
                        index += 1
                    found[{"H": "--head", "R": "--repo"}.get(letter, letter)] = value
                    break
                if letter not in PR_CREATE_SHORT_FLAGS:
                    raise ValueError("gh pr create flag `%s` is not one the gate knows" % arg)
    return found.get("--head"), found.get("--repo")


def origin_repo():
    """OWNER/REPO of the origin remote on GitHub, lower-cased, or None."""
    url = git_output("remote", "get-url", "origin") or ""
    match = re.search(r"github\.com[:/]([^/\s]+/[^/\s]+?)(?:\.git)?/?$", url)
    return match.group(1).lower() if match else None


def repo_slug(value):
    value = re.sub(r"^(?:https?://)?(?:www\.)?github\.com/", "", value.strip()).rstrip("/")
    return re.sub(r"\.git$", "", value).lower()


def api_arguments(args):
    """(endpoint, method or None, has_fields) from gh api's arguments, or raise Unparseable on a short flag
    the gate does not know.

    pflag accepts `-iX PUT`, `-iXPUT`, `-X=PUT` and `-fk=v`, so short bundles are walked letter by letter:
    reading `-iX` as a boolean once put PUT in the endpoint's place.
    """
    positional, method, fields, index = [], None, False, 0
    while index < len(args):
        arg = args[index]
        index += 1
        if arg == "--":
            positional.extend(args[index:])
            break
        if arg.startswith("--"):
            name, equals, value = arg.partition("=")
            if name in API_LONG_VALUES and not equals:
                value = args[index] if index < len(args) else ""
                index += 1
            if name == "--method":
                method = value
            fields = fields or name in ("--field", "--raw-field", "--input")
        elif arg.startswith("-") and len(arg) > 1:
            for position, letter in enumerate(arg[1:], 1):
                if letter in API_SHORT_VALUES:
                    value = arg[position + 1:]
                    if not value:
                        value = args[index] if index < len(args) else ""
                        index += 1
                    elif value.startswith("="):
                        value = value[1:]
                    if letter == "X":
                        method = value
                    fields = fields or letter in "fF"
                    break
                if letter not in API_SHORT_FLAGS:
                    raise Unparseable("gh api flag `%s` is not one the gate knows" % arg)
        else:
            positional.append(arg)
    return (positional[0] if positional else ""), method, fields


def api_calls(text):
    """(endpoint, method, args) for every `gh api` call; a call with fields defaults to POST, as gh does."""
    calls = []
    for args in invocations(text, ["gh", "api"]):
        endpoint, method, has_fields = api_arguments(args)
        calls.append((endpoint, (method or ("POST" if has_fields else "GET")).upper(), args))
    return calls


def api_pr_create(text):
    """A reason when `gh api` would open a PR (REST or GraphQL), else None."""
    for endpoint, method, args in api_calls(text):
        if api_endpoint(endpoint) == "graphql" and any("createpullrequest" in arg.lower() for arg in args):
            return "the GraphQL createPullRequest mutation opens a PR"
        if method != "GET" and API_PULLS.match(api_endpoint(endpoint)):
            return "`gh api %s` opens a PR" % endpoint
    return None


def api_endpoint(endpoint):
    """An endpoint as GitHub routes it: no host, leading slash, query string or fragment, lower-cased."""
    endpoint = API_HOST_PREFIX.sub("", endpoint.strip().lower())
    return re.split(r"[?#]", endpoint, maxsplit=1)[0].rstrip("/")


def api_fields(args):
    """(the `key=value` of every field gh api sends, whether --input supplies the body) from gh api's arguments,
    read as api_arguments reads them; a short flag the gate does not know raises Unparseable."""
    fields, has_input, index = [], False, 0
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
            if name in ("--field", "--raw-field"):
                fields.append(value)
            has_input = has_input or name == "--input"
        elif arg.startswith("-") and len(arg) > 1:
            for position, letter in enumerate(arg[1:], 1):
                if letter in API_SHORT_VALUES:
                    value = arg[position + 1:]
                    if not value:
                        value = args[index] if index < len(args) else ""
                        index += 1
                    elif value.startswith("="):
                        value = value[1:]
                    if letter in "fF":
                        fields.append(value)
                    break
                if letter not in API_SHORT_FLAGS:
                    raise Unparseable("gh api flag `%s` is not one the gate knows" % arg)
    return fields, has_input


GRAPHQL_HINT = "move a board card with bin/board-status.sh; a direct gh api graphql may only read, with a " \
               "single-quoted query document"


def shell_builds_text(text):
    """Whether the line holds text the shell rewrites before gh sees it: a `$` or backtick outside single quotes,
    ANSI-C quoting, or a glob, brace, tilde or process substitution outside any quotes. Heredoc bodies no shell
    reads are data and are not looked at."""
    code, shell_bodies = split_heredocs(drop_quoted_cat_blocks(text))
    for part in [code] + shell_bodies:
        stripped, quote, index = join_and_strip(part), None, 0
        if ANSI_C in stripped:
            return True
        while index < len(stripped):
            char = stripped[index]
            if quote == "'":
                quote = None if char == "'" else quote
            elif char == "\\":
                index += 1
            elif char in "$`":
                return True
            elif quote == '"':
                quote = None if char == '"' else quote
            elif char in "'\"":
                quote = char
            elif char in "*?[{}~" or stripped.startswith("<(", index):
                return True
            index += 1
    return False


def graphql_refusal(args, text):
    """Why one `gh api graphql` call must not run, or None. It passes only as a read: exactly one `query` field
    whose document is a literal on the line (no `@file`, stdin or --input, nothing the shell builds) and no
    `mutation` anywhere in its arguments, in any letter case. Brace and glob expansion, flag spellings and
    substitutions each hid a mutation from a parser, so anything the gate cannot read whole is refused."""
    fields, has_input = api_fields(args)
    if has_input:
        return "gh api graphql --input reads its document from a file or stdin the gate cannot see; " + GRAPHQL_HINT
    if any(field.partition("=")[2].startswith("@") for field in fields):
        return "gh api graphql reads a field from a file or stdin (`@`) the gate cannot see; " + GRAPHQL_HINT
    if [field.partition("=")[0] for field in fields].count("query") != 1:
        return "gh api graphql names no single literal query document; " + GRAPHQL_HINT
    if any("mutation" in arg.lower() for arg in args):
        return "gh api graphql runs a mutation; " + GRAPHQL_HINT
    if shell_builds_text(text):
        return "gh api graphql on a line whose text the shell rewrites (a variable, substitution, glob or brace) " \
               "cannot be shown to only read; " + GRAPHQL_HINT
    return None


def api_write(text):
    """A reason when `gh api` would write anything outside the allowlist, or call GraphQL other than to read.

    A denylist of merge endpoints misses the contents, refs and createCommitOnBranch paths onto next or main,
    so every REST write fails closed unless the workflow needs it. GraphQL passes only as a literal read
    (graphql_refusal); bin/board-status.sh makes the one mutation the workflow needs, inside the script.
    """
    for endpoint, method, args in api_calls(text):
        path = api_endpoint(endpoint)
        if path == "graphql":
            reason = graphql_refusal(args, text)
            if reason:
                return reason
            continue
        if method in ("GET", "HEAD"):
            continue
        if not any(method == allowed and pattern.match(path) for allowed, pattern in API_WRITE_ALLOWLIST):
            return "`gh api -X %s %s`" % (method, endpoint)
    return None


def push_checkout(text):
    """Where the line's push runs: `` for the hook's cwd, `=DIR` for `cd DIR && git push`, `?` for anywhere else."""
    if not moves_repository(text):
        return ""
    directory = cd_target(text, ["git", "push"])
    return "=%s" % directory if directory else "?"


def push_destinations(text):
    """The branch each push names as its destination: `feat/1/x` for `git push origin feat/1/x`, `HEAD:feat/1/x`
    or `+x:refs/heads/feat/1/x`. A deletion (`:x`, -d), a tag push, HEAD or @ (the current branch, which the
    caller reads itself) and a name the shell builds are skipped."""
    found = []
    for args in invocations(text, ["git", "push"]):
        if any(arg in ("--delete", "--tags") or re.fullmatch(r"-[A-Za-z]*d[A-Za-z]*", arg) for arg in args):
            continue
        for refspec in positionals(args, PUSH_VALUE_OPTIONS)[1:]:
            source, colon, destination = refspec.lstrip("+").rpartition(":")
            if (colon and not source) or not destination or destination in CURRENT_HEAD \
                    or UNRESOLVED_REF.search(destination) or destination.startswith("refs/tags/"):
                continue
            for prefix in ("refs/heads/", "heads/"):
                if destination.startswith(prefix):
                    destination = destination[len(prefix):]
                    break
            found.append(destination)
    return found


def pr_head(text):
    """`=BRANCH` the line opens a PR from (`=` for the current branch), `?REASON`, or None when it opens none."""
    reason = api_pr_create(text)
    if reason:
        return "?%s, which bypasses the PR gates; use gh pr create" % reason
    heads = set()
    for args in invocations(text, ["gh", "pr", "create"]):
        try:
            head, repo = pr_create_options(args)
        except ValueError as error:
            return "?%s" % error
        origin = origin_repo()
        if repo is not None and (origin is None or repo_slug(repo) != origin):
            return "?--repo %s is not this checkout's origin, whose commits the markers describe" % repo
        if head and ":" in head:
            owner, _, head = head.partition(":")
            if origin is None or owner.lower() != origin.split("/")[0]:
                return "?--head %s:%s is on a fork the markers cannot describe" % (owner, head)
        if head and (head.startswith("-") or not re.fullmatch(r"[A-Za-z0-9._/-]+", head)):
            return "?--head `%s` is not a branch name the gate can resolve" % head
        heads.add(head or "")
    if re.search(r"\bGH_REPO=", text) and heads:
        return "?GH_REPO on the line points gh at a repository the gate cannot check"
    # The gates follow only `cd DIR && gh pr create`; any other move would leave them checking the session cwd.
    if heads and moves_repository(text) and cd_target(text, ["gh", "pr", "create"]) is None:
        return "?this line changes directory or repository, so the gate cannot tell which checkout the PR opens " \
               "from; run gh pr create alone or as `cd DIR && gh pr create`"
    if len(heads) > 1:
        return "?this line opens PRs from more than one branch"
    return "=%s" % heads.pop() if heads else None


BRANCH_COMMANDS = ("switch", "checkout", "branch")
# git branch modes that list, delete or set tracking: none of them creates, renames or copies a branch.
BRANCH_LIST_OR_DELETE = {"--delete", "--list", "--all", "--remotes", "--verbose", "--show-current", "--contains",
                         "--no-contains", "--merged", "--no-merged", "--points-at", "--set-upstream-to",
                         "--unset-upstream", "--edit-description", "--format", "--sort"}
GIT_DIRECTORY_ENV = re.compile(r"GIT_(?:DIR|WORK_TREE)=")


def literal_directory(base, directory):
    """DIRECTORY resolved against BASE when the shell takes it as written and it exists, else None. A cd that
    fails leaves the shell where it was (`cd /missing; git switch -c x` runs in the cwd), so a directory
    that is not there is as unknown as a variable."""
    if base is None or not directory or re.search(r"[$`*?\[\\~{}\x01]", directory):
        return None
    resolved = os.path.normpath(os.path.join(base, directory))
    return resolved if os.path.isdir(resolved) else None


def git_directory(words, base):
    """Where a top-level `git ...` runs: BASE moved by its literal -C options, or None when a -C, --git-dir,
    --work-tree or a GIT_DIR/GIT_WORK_TREE prefix makes it unknowable."""
    command = strip_prefix(words)
    if any(GIT_DIRECTORY_ENV.match(word) for word in words[:len(words) - len(command)]) or \
            env_changes_directory(words[:len(words) - len(command)]):
        return None
    index = 1
    while index < len(command) and command[index].startswith("-"):
        option = command[index]
        if option == "-C":
            base = literal_directory(base, command[index + 1] if index + 1 < len(command) else "")
            index += 2
            continue
        if option.startswith(("--git-dir", "--work-tree")):
            return None
        index += 2 if option in ("-c", "--namespace", "--super-prefix", "--config-env", "--attr-source") else 1
    return base


def git_command_sites(text, cwd, subs):
    """(directory or None, subcommand, arguments) for every `git SUB` in SUBS the line runs. A top-level
    command's directory follows the cd, pushd and -C before it (None once one is not a literal path); one run
    inside another command (bash -c, xargs, eval) runs in CWD unless the line changes directory."""
    sites, directory = [], cwd
    groups = simple_commands(join_and_strip(split_heredocs(drop_quoted_cat_blocks(text))[0]))
    for raw, _here, _operator in groups:
        words = shell_words(raw)
        command = strip_prefix(words)
        if not command:
            continue
        name = os.path.basename(command[0])
        if name in ("cd", "pushd"):
            directory = literal_directory(directory, command[1]) if len(command) == 2 else None
        elif name == "popd":
            directory = None
        elif runs_git(command[0]):
            git = as_git(command)
            if git[1:2] and git[1] in subs:
                sites.append((git_directory(words, directory), git[1], git[2:]))
    for sub in subs:
        found = invocations(text, ["git", sub])
        if len(found) > len([site for site in sites if site[1] == sub]):
            nested_directory = None if moves_repository(text) else cwd
            sites += [(nested_directory, sub, args) for args in found]
    return sites


def is_primary_checkout(directory):
    """Whether DIRECTORY is inside the primary checkout (its git dir is the common dir), not a linked worktree."""
    dirs = git_output("-C", directory, "rev-parse", "--path-format=absolute", "--git-dir", "--git-common-dir")
    if not dirs or len(dirs.split("\n")) != 2:
        return False
    git_dir, common_dir = dirs.split("\n")
    return os.path.realpath(git_dir) == os.path.realpath(common_dir)


def names_commit(directory, ref):
    """Whether `git checkout REF` would switch to REF (a commit, or a branch git creates from origin/REF)
    rather than restore a file named REF."""
    return bool(git_output("-C", directory, "rev-parse", "--verify", "--quiet", ref + "^{commit}")
                or git_output("-C", directory, "rev-parse", "--verify", "--quiet", "refs/remotes/origin/" + ref))


def changes_branch(sub, args, directory):
    """Whether `git SUB ARGS` run in DIRECTORY creates, renames, copies or switches a branch. DIRECTORY None
    (unknown) reads a checkout's target as a branch, so the caller fails closed."""
    current = git_output("-C", directory, "symbolic-ref", "--quiet", "--short", "HEAD") if directory else None
    if sub == "branch":
        shorts = set("".join(arg[1:] for arg in args if re.fullmatch(r"-[A-Za-z]+", arg)))
        longs = {arg.split("=", 1)[0] for arg in args if arg.startswith("--")}
        if shorts & set("mMcC") or any(long_option(arg, {"--move", "--copy"}) for arg in args):
            return True
        if shorts & set("dDlarvu") or longs & BRANCH_LIST_OR_DELETE:
            return False
        return any(not arg.startswith("-") for arg in args)
    if sub == "switch":
        positional = []
        for arg in args:
            if arg in ("-c", "-C", "--create", "--force-create", "--orphan", "--detach", "-d") or \
                    re.fullmatch(r"-[A-Za-z]*[cCd][A-Za-z]*", arg) or arg.startswith(("--create=", "--orphan=")):
                return True
            if arg == "-" or not arg.startswith("-"):
                positional.append(arg)
        return bool(positional) and positional[0] != current
    if "--" in args:
        dashes = args.index("--")
        if args[dashes + 1:] and not any(arg in ("-b", "-B", "--orphan", "--detach") or
                                         re.fullmatch(r"-[A-Za-z]*[bB][A-Za-z]*", arg) for arg in args[:dashes]):
            return False  # `git checkout [<tree-ish>] -- <paths>` restores files
        args = args[:dashes]  # a bare trailing `--` only marks the tree-ish as a revision: it still switches
    positional = []
    for arg in args:
        if arg in ("-b", "-B", "--orphan", "--detach") or re.fullmatch(r"-[A-Za-z]*[bB][A-Za-z]*", arg):
            return True
        if arg == "-" or not arg.startswith("-"):
            positional.append(arg)
    if not positional or positional[0] == current:
        return False
    if len(positional) > 1:
        return False  # `git checkout <tree-ish> <pathspec>...` restores files, as with --
    return directory is None or positional[0] == "-" or names_commit(directory, positional[0])


RESTORE_VALUE_OPTIONS = {"-s", "--source", "--pathspec-from-file", "--conflict"}
CHECKOUT_VALUE_OPTIONS = {"--pathspec-from-file", "--conflict"}


def restore_source_and_paths(sub, args):
    """(tree-ish or None, pathspecs) of a `git checkout` or `git restore` that restores files, or None when it is
    not one (a checkout that switches). A restore with no tree-ish takes the index; --pathspec-from-file adds
    the unknown pathspec `?`."""
    source, paths, index, dashes, from_file = None, [], 0, False, False
    values = RESTORE_VALUE_OPTIONS if sub == "restore" else CHECKOUT_VALUE_OPTIONS
    while index < len(args):
        arg = args[index]
        index += 1
        if dashes or not arg.startswith("-") or arg == "-":
            paths.append(arg)
            continue
        if arg == "--":
            dashes = True
            if sub == "checkout" and len(paths) == 1:
                source, paths = paths[0], []  # `git checkout TREE -- paths`
            continue
        name, equals, value = arg.partition("=")
        if sub == "restore" and long_option(name, {"--source"}):
            if not equals:
                value = args[index] if index < len(args) else ""
                index += 1
            source = value
            continue
        bundle = re.match(r"-([A-Za-z0-9]*?)s(.*)$", arg, re.DOTALL) if sub == "restore" and arg[1] != "-" else None
        if bundle:
            rest = bundle.group(2)  # -s, restore's one short option with a value, takes the rest of its bundle
            if not rest:
                rest = args[index] if index < len(args) else ""
                index += 1
            source = rest
            continue
        from_file = from_file or long_option(name, {"--pathspec-from-file"}) is not None
        if not equals and name in values:
            index += 1
    if sub == "checkout" and not dashes:
        if len(paths) < 2:
            return None
        source, paths = paths[0], paths[1:]
    return source, paths + (["?"] if from_file else [])


def covers_hooks(directory, path):
    """Whether pathspec PATH, given where git runs (None: unknown), could reach .claude/ (the hooks every session
    runs): the top, .claude itself or below it, a glob or other pathspec magic, a path outside the tree, or `?`."""
    if directory is None or path == "?" or re.search(r"[*?\[]", path):
        return True
    if path.startswith(":/"):
        full = os.path.normpath(path[2:] or ".")
    elif path.startswith(":"):
        return True
    else:
        prefix = git_output("-C", directory, "rev-parse", "--show-prefix")
        if prefix is None:
            return True
        full = os.path.normpath(os.path.join(prefix, path))
    return full in (".", ".claude") or full.startswith((".claude/", ".."))


def hook_overwrite(sub, args, directory):
    """A reason a `git checkout`/`git restore` in the primary checkout DIRECTORY would write .claude/ from a
    tree-ish other than HEAD, or None. From HEAD or the index it only discards local edits."""
    found = restore_source_and_paths(sub, args)
    if not found:
        return None
    source, paths = found
    if source is None or source in CURRENT_HEAD:
        return None
    reaching = [path for path in paths if covers_hooks(directory, path)]
    if not reaching:
        return None
    return "`git %s %s` would overwrite the primary checkout's .claude/ (the hooks every session runs) from %s" % (
        sub, " ".join(args), source)


def is_pr_checkout(command):
    """Whether COMMAND (`gh ...`, prefix stripped) is `gh pr checkout` in any spelling gh accepts: -R before the
    group or the subcommand, or the built-in `gh co` alias."""
    words = gh_subcommand_first(command)
    return words[1:3] == ["pr", "checkout"] or words[1:2] == ["co"]


def pr_checkout_sites(text, cwd):
    """The directory (None when unknown) of every `gh pr checkout` the line runs. gh switches the branch of the
    checkout it runs in, whatever -R names. A top-level one follows the cd and pushd before it; one run inside
    another command (bash -c, xargs, eval) runs in CWD unless the line changes directory."""
    sites, directory = [], cwd
    groups = simple_commands(join_and_strip(split_heredocs(drop_quoted_cat_blocks(text))[0]))
    for raw, _here, _operator in groups:
        words = shell_words(raw)
        command = strip_prefix(words)
        if not command:
            continue
        name = os.path.basename(command[0])
        if name in ("cd", "pushd"):
            directory = literal_directory(directory, command[1]) if len(command) == 2 else None
        elif name == "popd":
            directory = None
        elif name == "gh" and is_pr_checkout(command):
            prefix = words[:len(words) - len(command)]
            sites.append(None if env_changes_directory(prefix) else directory)
    found = len(invocations(text, ["gh", "pr", "checkout"])) + len(invocations(text, ["gh", "co"]))
    if found > len(sites):
        sites += [None if moves_repository(text) else cwd] * (found - len(sites))
    return sites


def primary_branch_change(text, cwd=None):
    """A reason the line would create, rename, copy or switch a branch in the primary checkout, or None. The
    primary checkout stays on next; a branch is born in a worktree (`git worktree add -b`), and /start renames
    a worktree-* branch with `git branch -m` inside the worktree, which passes."""
    cwd = cwd or os.getcwd()
    for directory, sub, args in git_command_sites(text, cwd, BRANCH_COMMANDS):
        if not changes_branch(sub, args, directory):
            continue
        if directory is None:
            return "`git %s` changes a branch in a checkout the gate cannot tell (the line changes directory or " \
                   "repository); run it from inside the worktree, or as `cd /abs/worktree && git %s ...`" % (sub, sub)
        if is_primary_checkout(directory):
            return "`git %s %s` would create, rename or switch a branch in the primary checkout (%s), which stays " \
                   "on next" % (sub, " ".join(args), directory)
    for directory, sub, args in git_command_sites(text, cwd, ("checkout", "restore")):
        if directory is None or is_primary_checkout(directory):
            reason = hook_overwrite(sub, args, directory)
            if reason:
                return reason + ("" if directory else " (the line changes directory, so the gate cannot tell "
                                 "it is not the primary checkout)")
    for directory in pr_checkout_sites(text, cwd):
        if directory is None:
            return "`gh pr checkout` switches the branch of a checkout the gate cannot tell (the line changes " \
                   "directory); run it from inside the worktree, or as `cd /abs/worktree && gh pr checkout N`"
        if is_primary_checkout(directory):
            return "`gh pr checkout` would switch the primary checkout (%s) off next" % directory
    return None


def staged_migrations(text, cwd=None):
    """For migration-gate.sh: `commit DIR` (`commit ?` when the gate cannot tell where) for each git commit
    the line runs, and `add PATH` for each path a git add on the line names (relative to the repository's top
    when the add runs in a known subdirectory), one per line."""
    cwd = cwd or os.getcwd()
    lines = []
    for directory, sub, args in git_command_sites(text, cwd, ("commit", "add", "stage")):
        if sub == "commit":
            lines.append("commit %s" % (directory or "?"))
            continue
        # A path is relative to where git add runs; resolved against the repository's top when that is known.
        prefix = git_output("-C", directory, "rev-parse", "--show-prefix") if directory else None
        for arg in args:
            if arg.startswith("-") or "\n" in arg:
                continue
            lines.append("add %s" % (os.path.normpath(os.path.join(prefix, arg))
                                     if prefix is not None and not arg.startswith(":") else arg))
    return lines


# git commit's short options that take a value (the rest of the bundle, else the next word), the ones whose value
# can only be attached (-u<mode>, -S<keyid>), and its long options that take a value.
COMMIT_SHORT_VALUES = set("mFCct")
COMMIT_SHORT_ATTACHED = set("uS")
COMMIT_LONG_VALUES = {"--message", "--file", "--reuse-message", "--reedit-message", "--template", "--author",
                      "--date", "--cleanup", "--fixup", "--squash", "--trailer", "--pathspec-from-file"}
WHOLE_TREE_PATHSPECS = (".", "./", ":/", ":/.", ":(top)", "*", "./*", ":/*")
# `:!x`, `:^x`, `:(exclude)x`, `:(top,exclude)x`: a pathspec that only subtracts. A list of nothing else matches
# everything but those paths.
EXCLUDE_PATHSPEC = re.compile(r"^:(?:[!^]|\([^)]*\bexclude\b[^)]*\))")


def only_exclusions(pathspecs):
    """Whether PATHSPECS is non-empty and every one of them is an exclusion."""
    return bool(pathspecs) and all(EXCLUDE_PATHSPEC.match(path) for path in pathspecs)


def commit_stages_all(args):
    """The spelling of -a/--all in `git commit ARGS`, read as git's parse-options reads it, or of a pathspec
    list that commits the whole tree (`.`, `:/`, `*`, or exclusions only), or None. In a bundle the letters
    before a value letter are flags: `-am x` is -a then -m x, `-ma` is -m with the message `a`."""
    index, pathspecs = 0, []
    while index < len(args):
        arg = args[index]
        index += 1
        if arg == "--":
            pathspecs += args[index:]
            break
        if not arg.startswith("-") or arg == "-":
            pathspecs.append(arg)
            continue
        if arg.startswith("--"):
            name = arg.split("=", 1)[0]
            if len(name) >= 3 and "--all".startswith(name):
                return arg
            if "=" not in arg and long_option(arg, COMMIT_LONG_VALUES):
                index += 1
        elif arg.startswith("-") and len(arg) > 1:
            for position, letter in enumerate(arg[1:], 1):
                if letter == "a":
                    return arg
                if letter in COMMIT_SHORT_ATTACHED:
                    break
                if letter in COMMIT_SHORT_VALUES:
                    index += 0 if arg[position + 1:] else 1
                    break
    whole = [path for path in pathspecs if path in WHOLE_TREE_PATHSPECS]
    if whole:
        return whole[0]
    if only_exclusions(pathspecs):
        return " ".join(pathspecs)
    return None


def stages_everything(text):
    """A reason the line stages the whole tree, else None: `git add` (or `git stage`) with -A/--all in any
    spelling git accepts, --no-ignore-removal, -u/--update with no pathspec, a pathspec of `.`, `./`, `:/`,
    `:/.` or `*`, or pathspecs that only exclude; or `git commit` with -a/--all or such a pathspec list, which
    commits every tracked change (conflict markers included, mid-merge)."""
    for sub in ("add", "stage"):
        for args in invocations(text, ["git", sub]):
            after_dashes, update, pathspec, pathspecs = False, None, False, []
            for arg in args:
                if arg == "--" and not after_dashes:
                    after_dashes = True
                    continue
                name = arg.split("=", 1)[0]
                if not after_dashes and arg.startswith("--"):
                    if (len(name) >= 3 and "--all".startswith(name)) or \
                            (len(name) >= 12 and "--no-ignore-removal".startswith(name)):
                        return "`git %s %s` stages everything in the tree" % (sub, arg)
                    if len(name) >= 3 and "--update".startswith(name):
                        update = arg
                    pathspec = pathspec or long_option(arg, {"--pathspec-from-file"}) is not None
                elif not after_dashes and re.fullmatch(r"-[A-Za-z]+", arg):
                    if "A" in arg:
                        return "`git %s %s` stages everything in the tree" % (sub, arg)
                    if "u" in arg:
                        update = arg
                elif (after_dashes or not arg.startswith("-")) and arg in WHOLE_TREE_PATHSPECS:
                    return "`git %s %s` stages everything in the tree" % (sub, arg)
                elif after_dashes or not arg.startswith("-"):
                    pathspec = True
                    pathspecs.append(arg)
            if only_exclusions(pathspecs):
                return "`git %s %s` excludes paths and nothing else, so it stages everything else in the tree" % (
                    sub, " ".join(pathspecs))
            if update and not pathspec:
                return "`git %s %s` with no pathspec stages every tracked change in the tree" % (sub, update)
    for args in invocations(text, ["git", "commit"]):
        flag = commit_stages_all(args)
        if flag:
            return "`git commit %s` commits every tracked change in the tree, unreviewed (and conflict markers, " \
                   "mid-merge)" % flag
    return None


def main():
    if len(sys.argv) < 2:
        print("usage: git_policy.py protected-push BRANCH | non-main-integration | update-branch | pr-head"
              " | api-write | push-checkout | push-destinations | primary-branch | stage-all | migrations", file=sys.stderr)
        return 2
    text = sys.stdin.read()
    try:
        if sys.argv[1] == "update-branch":
            for target in update_branch_targets(text):
                print(target)
            return 0
        if sys.argv[1] == "push-checkout":
            print(push_checkout(text))
            return 0
        if sys.argv[1] == "migrations":
            for line in staged_migrations(text):
                print(line)
            return 0
        if sys.argv[1] == "push-destinations":
            for destination in push_destinations(text):
                print(destination)
            return 0
        if sys.argv[1] == "protected-push" and len(sys.argv) == 3:
            reason = protected_push(text, sys.argv[2])
        elif sys.argv[1] == "non-main-integration":
            reason = non_main_integration(text)
        elif sys.argv[1] == "pr-head":
            reason = pr_head(text)
        elif sys.argv[1] == "api-write":
            reason = api_write(text)
        elif sys.argv[1] == "primary-branch":
            reason = primary_branch_change(text)
        elif sys.argv[1] == "stage-all":
            reason = stages_everything(text)
        else:
            print("git_policy: unknown policy %r" % sys.argv[1], file=sys.stderr)
            return 2
    except Unparseable as error:
        print("git_policy: %s" % error, file=sys.stderr)
        return 2
    if reason:
        print(reason)
    return 0


if __name__ == "__main__":
    sys.exit(main())
