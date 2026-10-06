# Split a shell command line into the simple commands it runs, one per output line.
#
# The gates match on what a command *does*, so they must not match on what it *says*: a commit
# message or PR body that mentions "git push origin next" is not a push. This is not a shell parser,
# only enough of one for that:
#   * heredoc bodies are dropped (the `<<'EOF'` line stays, its body and terminator go);
#   * a quoted string becomes its bare text when it is one plain word (`"next"` -> next), and the
#     placeholder __Q__ otherwise, so a quoted message never splits into fake commands;
#   * `;` `&` `|` `(` `)` newlines, backticks and `$(` end a command, so `a && git push` and
#     `echo $(git push)` both expose the push;
#   * redirections (`> f`, `2>&1`, `<<'EOF'`) and comments are dropped.
# POSIX awk only: it runs under macOS's BSD awk as well as gawk/mawk.

{ lines[++n] = $0 }

END {
    text = ""
    delim = ""
    for (i = 1; i <= n; i++) {
        line = lines[i]
        if (delim != "") {
            t = line
            sub(/^[ \t]+/, "", t)
            sub(/[ \t]+$/, "", t)
            if (t == delim) delim = ""
            continue
        }
        text = text line "\n"
        if (match(line, /<<-?[ \t]*['"]?[A-Za-z_][A-Za-z0-9_]*['"]?/)) {
            # `<<<word` is a here-string, not a heredoc: match() lands on its second `<`.
            if (RSTART == 1 || substr(line, RSTART - 1, 1) != "<") {
                d = substr(line, RSTART, RLENGTH)
                sub(/^<<-?[ \t]*['"]?/, "", d)
                sub(/['"]$/, "", d)
                delim = d
            }
        }
    }

    seg = ""
    L = length(text)
    i = 1
    while (i <= L) {
        c = substr(text, i, 1)
        if (c == "\\") {
            nx = substr(text, i + 1, 1)
            seg = seg (nx == "\n" ? " " : nx)
            i += 2
            continue
        }
        if (c == "'" || c == "\"") {
            i = read_quoted(i)
            seg = seg plain(word)
            continue
        }
        if (c == "$" && substr(text, i + 1, 1) == "(") {
            emit()
            i += 2
            continue
        }
        if (c == ";" || c == "&" || c == "|" || c == "(" || c == ")" || c == "\n" || c == "`") {
            emit()
            i++
            continue
        }
        if (c == ">" || c == "<") {
            # Drop the fd number glued in front (`2>`), the operator, and its target word.
            sub(/[0-9]+$/, "", seg)
            while (i <= L && index("<>&-", substr(text, i, 1)) > 0) i++
            while (i <= L && (substr(text, i, 1) == " " || substr(text, i, 1) == "\t")) i++
            i = skip_word(i)
            continue
        }
        if (c == "#" && (seg == "" || substr(seg, length(seg), 1) == " " || substr(seg, length(seg), 1) == "\t")) {
            while (i <= L && substr(text, i, 1) != "\n") i++
            continue
        }
        seg = seg c
        i++
    }
    emit()
}

# Reads the quoted string opening at text[i] into `word`; returns the index after its closing quote.
function read_quoted(i,    q, ch) {
    q = substr(text, i, 1)
    word = ""
    i++
    while (i <= L) {
        ch = substr(text, i, 1)
        if (ch == q) return i + 1
        if (q == "\"" && ch == "\\") {
            word = word substr(text, i + 1, 1)
            i += 2
            continue
        }
        word = word ch
        i++
    }
    return i
}

# Skips one shell word (quotes included) starting at text[i]; returns the index after it.
function skip_word(i,    ch) {
    while (i <= L) {
        ch = substr(text, i, 1)
        if (ch == "'" || ch == "\"") {
            i = read_quoted(i)
            continue
        }
        if (ch == " " || ch == "\t" || index(";&|()\n`", ch) > 0) return i
        i++
    }
    return i
}

function plain(w) {
    if (w ~ /^[A-Za-z0-9_.\/:@%+=,~^-]*$/) return w
    return "__Q__"
}

function emit(    s) {
    s = seg
    gsub(/^[ \t]+/, "", s)
    gsub(/[ \t]+$/, "", s)
    if (s != "") print s
    seg = ""
}
