"""Deterministic extraction of filesystem operations from one shell command, and the resolver that
both extractors share: raw operations (paths as written) -> candidates resolved against cwd.

An operation is {"op", "src", "dst"}; `cd` only moves the resolver, and `base` (git -C) resolves
one operation against another directory. Nothing here runs or expands a command."""
import ntpath
from dataclasses import dataclass
import re

CANDIDATE_OPS = {"create", "copy", "move", "clone", "worktree", "write"}


class Op(str):
    """An operator token (`&&`, `|`, `>`, `,` ...), never a word."""


OPERATORS = {
    "bash": ["&&", "||", "|&", "&>", ">>", ">&", "<<-", "<<", ";", "|", "&", ">", "<", "(", ")"],
    "powershell": ["&&", "||", ">>", ">&", ";", "|", ">", "<", "(", ")", "{", "}", ","],
    "cmd": ["&&", "||", ">>", ">&", "&", "|", ">", "<", "(", ")"],
}
ESCAPE = {"bash": "\\", "powershell": "`", "cmd": "^"}
SEPARATORS = {"&&", "||", "|&", ";", "|", "&", "(", ")", "{", "}"}
REDIRECT_OUT = {">", ">>", "&>"}


def scan(cmd, shell):
    """Words and operators, with the shell's own quoting. Heredoc bodies are dropped."""
    toks, word, has, i, n, pending = [], [], False, 0, len(cmd), []
    ops, esc = OPERATORS[shell], ESCAPE[shell]

    def flush():
        nonlocal word, has
        if has:
            toks.append("".join(word))
        word, has = [], False

    while i < n:
        c = cmd[i]
        if c in " \t\r":
            flush(); i += 1; continue
        if c == "\n":
            flush(); toks.append(Op(";")); i += 1
            for delim, tok in pending:  # heredoc bodies start on the next line: each ends at its delimiter
                body = re.search(rf"^\t*{re.escape(delim)}[ \t]*$", cmd[i:], re.M)
                tok.body = cmd[i:] if body is None else cmd[i:i + body.start()]
                i = n if body is None else i + body.end()
            pending.clear()
            continue
        if c == esc and i + 1 < n:
            if cmd[i + 1] == "\n":
                i += 2; continue
            word.append(cmd[i + 1]); has = True; i += 2; continue
        if c == "'" and shell != "cmd":
            j = i + 1
            while True:
                k = cmd.find("'", j)
                if k < 0:
                    k = n; break
                if shell == "powershell" and cmd[k + 1:k + 2] == "'":
                    word.append(cmd[j:k + 1]); j = k + 2; continue
                break
            word.append(cmd[j:k]); has = True; i = k + 1; continue
        if c == '"':
            j = i + 1
            while j < n and cmd[j] != '"':
                if shell == "bash" and cmd[j] == "\\" and cmd[j + 1:j + 2] in ('"', "\\", "$", "`"):
                    word.append(cmd[j + 1]); j += 2; continue
                if shell == "powershell" and cmd[j] == "`" and j + 1 < n:
                    word.append(cmd[j + 1]); j += 2; continue
                word.append(cmd[j]); j += 1
            has = True; i = j + 1; continue
        if c == "$" and cmd[i + 1:i + 2] == "(":  # $(...) stays inside its word
            depth, j = 0, i + 1
            while j < n:
                depth += {"(": 1, ")": -1}.get(cmd[j], 0)
                j += 1
                if depth == 0:
                    break
            word.append(cmd[i:j]); has = True; i = j; continue
        if not has and (c.isdigit() or c == "*") and cmd[i + 1:i + 2] == ">":
            i += 1; continue  # 2> and *> redirect like >
        o = next((o for o in ops if cmd.startswith(o, i)), None)
        if o is None:
            word.append(c); has = True; i += 1; continue
        flush(); i += len(o)
        if o in ("<<", "<<-"):  # the rest of the line goes on; the body is skipped at the newline
            m = re.match(r"\s*(['\"]?)([\w.-]+)\1", cmd[i:])
            tok = Op("<<")
            tok.body = None
            if m:
                pending.append((m[2], tok)); i += m.end()
            toks.append(tok)
            continue
        toks.append(Op(o))
    flush()
    return toks


def segments(toks):
    """Simple commands: (words, redirect targets). `,` stays in the words for PowerShell arrays."""
    out, words, redirs, i = [], [], [], 0
    while i < len(toks):
        t = toks[i]
        nxt = toks[i + 1] if i + 1 < len(toks) and not isinstance(toks[i + 1], Op) else None
        if isinstance(t, Op) and t in SEPARATORS:
            out.append((words, redirs)); words, redirs = [], []
        elif isinstance(t, Op) and t in REDIRECT_OUT:
            if nxt is not None:
                redirs.append(nxt); i += 1
        elif isinstance(t, Op) and t in ("<", ">&"):
            i += 1 if nxt is not None else 0
        else:
            words.append(t)
        i += 1
    out.append((words, redirs))
    return [s for s in out if s[0] or s[1]]


def op(kind, dst=None, src=None, **extra):
    return {"op": kind, "src": src, "dst": dst, **extra}


def words(a):
    return [x for x in a if not isinstance(x, Op)]


def posix(args, values=()):
    """Positional arguments and options; `values` are the options that take the next word."""
    pos, opts, a, i = [], {}, words(args), 0
    while i < len(a):
        x = a[i]
        if x == "--":
            pos += a[i + 1:]; break
        if x.startswith("--") and "=" in x:
            k, v = x.split("=", 1); opts[k] = v
        elif x.startswith("-") and len(x) > 1:
            if x in values and i + 1 < len(a):
                opts[x] = a[i + 1]; i += 1
            else:
                opts[x] = True
        else:
            pos.append(x)
        i += 1
    return pos, opts


def ps(args, values):
    """PowerShell parameters: prefix matching, `-Name:value`, comma arrays. `values` maps each
    accepted name or alias to its canonical key; any other parameter is a switch."""
    pos, opts, i = [], {}, 0

    def take(j):
        vals = [args[j]]; j += 1
        while j + 1 < len(args) and args[j] == "," and not isinstance(args[j + 1], Op):
            vals.append(args[j + 1]); j += 2
        return vals, j

    while i < len(args):
        x = args[i]
        if isinstance(x, Op):
            i += 1; continue
        if x.startswith("-") and len(x) > 1 and x[1].isalpha():
            name, _, inline = x[1:].partition(":")
            name = name.lower()
            keys = {values[name]} if name in values else {v for k, v in values.items() if k.startswith(name)}
            key = keys.pop() if len(keys) == 1 else None
            if key and inline:
                opts[key] = [inline]; i += 1
            elif key and i + 1 < len(args) and not isinstance(args[i + 1], Op):
                opts[key], i = take(i + 1)
            else:
                opts["-" + name] = True; i += 1  # a switch, or a value parameter left without its value
        else:
            vals, i = take(i); pos.append(vals)
    return pos, opts


PATH = {"path": "path", "literalpath": "path", "lp": "path", "pspath": "path"}
NEW_ITEM = {**PATH, "name": "name", "itemtype": "itemtype", "type": "itemtype", "it": "itemtype", "value": "value"}
COPY_ITEM = {**PATH, "destination": "dst", "filter": "x", "include": "x", "exclude": "x"}
FILE_PARAMS = {**PATH, "filepath": "path", "value": "x", "encoding": "x", "width": "x", "inputobject": "x",
               "stream": "x", "filter": "x", "include": "x", "exclude": "x"}
GIT_CLONE_VALUES = {"-b", "--branch", "--depth", "-o", "--origin", "-u", "--upload-pack", "--reference",
                    "--reference-if-able", "--separate-git-dir", "-c", "--config", "--filter", "-j",
                    "--jobs", "--template", "--server-option", "--shallow-since", "--shallow-exclude",
                    "--bundle-uri", "--revision"}
OUTPUT_FLAGS = {"--target-dir", "--basetemp", "--out", "--output", "--out-dir", "--outdir",
                "--output-dir", "--output-directory"}
LOOPS = {"for", "foreach", "foreach-object", "%"}
PREFIXES = {"sudo", "env", "time", "command", "exec", "nohup", "do", "then", "else", "elif", "if", "while",
            "until", "&", "!", "{", "call", "start"}
ASSIGNMENT = re.compile(r"^[A-Za-z_]\w*=")


def f_cd(a, sh, push=False):
    if sh == "powershell":
        pos, o = ps(a, PATH)
        p = (o.get("path") or (pos[0] if pos else [None]))[0]
    else:
        w = [x for x in words(a) if not (sh == "cmd" and x.lower() == "/d")]
        p = w[0] if w else ("~" if sh == "bash" else None)
    return [op("cd", dst=p, push=push)] if p is not None else []


def f_new_item(a, sh, kind="file"):
    pos, o = ps(a, NEW_ITEM)
    paths = o.get("path") or (pos[0] if pos else ["."] if "name" in o else [])
    if "name" in o:
        paths = [p + "\\" + o["name"][0] for p in paths]
    kind = str((o.get("itemtype") or [kind])[0]).lower()
    if kind.startswith("dir"):
        return [op("create", dst=p) for p in paths]
    return [op("write", dst=p) for p in paths] if kind.startswith("file") else []


def f_mkdir(a, sh):
    if sh == "powershell":
        return f_new_item(a, sh, "directory")
    if sh == "cmd":
        return [op("create", dst=x) for x in words(a) if not x.startswith("/")]
    return [op("create", dst=x) for x in posix(a, {"-m", "--mode"})[0]]


def f_copy(a, sh, kind):
    if sh == "powershell":
        pos, o = ps(a, COPY_ITEM)
        srcs = o["path"] if "path" in o else (pos.pop(0) if pos else [])
        dst = o["dst"][0] if "dst" in o else (pos[0][0] if pos else None)
    elif sh == "cmd":
        w = [x for x in words(a) if not x.startswith("/")]
        srcs, dst = w[:1], (w[1] if len(w) > 1 else None)
    else:
        pos, o = posix(a, {"-t", "--target-directory", "-S", "--suffix"})
        dst = o.get("-t") or o.get("--target-directory")
        srcs, dst = (pos, dst) if dst else (pos[:-1], pos[-1] if len(pos) > 1 else None)
    return [op(kind, src=s, dst=dst) for s in srcs] if dst else []


def f_robocopy(a, sh, kind="copy"):  # robocopy and xcopy: src dst [masks] /flags, from any shell
    return f_copy(a, "cmd", kind)


def f_git(a, sh):
    w, base, i = words(a), None, 0
    while i < len(w) and w[i].startswith("-"):
        if w[i] == "-C" and i + 1 < len(w):
            base = w[i + 1]; i += 2
        elif w[i] in ("-c", "--git-dir", "--work-tree", "--namespace") and i + 1 < len(w):
            i += 2
        else:
            i += 1
    sub, rest = (w[i], w[i + 1:]) if i < len(w) else (None, [])
    if "--help" in rest or "-h" in rest:
        return []
    if sub == "clone":
        pos, _ = posix(rest, GIT_CLONE_VALUES)
        return [op("clone", src=pos[0], dst=pos[1] if len(pos) > 1 else None, base=base)] if pos else []
    if sub == "worktree" and rest[:1] == ["add"]:
        pos, _ = posix(rest[1:], {"-b", "-B", "--reason"})
        return [op("worktree", dst=pos[0], base=base)] if pos else []
    if sub == "init":
        pos, _ = posix(rest, {"--template", "--separate-git-dir", "-b", "--initial-branch", "--object-format"})
        return [op("create", dst=pos[0], base=base)] if pos else []
    return []


def f_cargo(a, sh):
    w = words(a)
    if w[:1] in (["new"], ["init"]):
        pos, _ = posix(w[1:], {"--name", "--vcs", "--edition", "--registry", "--color", "--config", "-Z"})
        return [op("create", dst=pos[0])] if pos else []
    return []


def f_npm(a, sh):  # npm/pnpm/yarn/bun create|init <package> <dir> [-- template options]
    w = words(a)
    w = w[:w.index("--")] if "--" in w else w
    if w[:1] in (["create"], ["init"]):
        pos, _ = posix(w[1:], {"--scope", "-w", "--workspace"})
        return [op("create", dst=pos[1])] if len(pos) > 1 else []
    return []


def f_npx(a, sh):
    w = words(a)
    pos, _ = posix(w[:w.index("--")] if "--" in w else w, {"-p", "--package"})
    if pos and re.match(r"(@[^/]+/)?create-", pos[0]):
        return [op("create", dst=pos[1])] if len(pos) > 1 else []
    return []


def f_uv(a, sh):
    w = words(a)
    if w[:1] == ["init"]:
        pos, _ = posix(w[1:], {"--name", "--python", "-p", "--build-backend", "--vcs", "--author-from"})
        return [op("create", dst=pos[0])] if pos else []
    if w[:1] == ["venv"]:
        pos, _ = posix(w[1:], {"--python", "-p", "--prompt"})
        return [op("create", dst=pos[0] if pos else ".venv")]
    return []


# ---- calls with a literal path inside Python and JavaScript source
#
# Two linear steps: find a call site, then read its arguments with quotes and brackets respected.
# Only a whole literal counts: an f-string, a ${} template, a format string, a concatenation or a
# variable gives nothing. The script stays flagged unsure: its other effects are not known.

@dataclass(frozen=True)
class Call:
    callee: str               # regex of the function name
    arg: int                  # index of the path argument
    kind: str                 # operation it performs
    then: str | None = None   # a method that must follow, as in Path("x").mkdir()
    mode: int | None = None   # index of a mode literal that must write (open(path, "w"))


CALLS = {
    "py": (
        Call(r"open", 0, "write", mode=1),
        Call(r"os\.makedirs|os\.mkdir", 0, "create"),
        Call(r"Path", 0, "create", then=r"\.mkdir\("),
        Call(r"Path", 0, "write", then=r"\.(?:write_text|write_bytes|touch)\("),
        Call(r"shutil\.copytree|shutil\.copy2?|shutil\.copyfile", 1, "copy"),
        Call(r"shutil\.move", 1, "move"),
    ),
    "js": (
        Call(r"(?:fs\.|fsp\.)?(?:writeFileSync|writeFile|appendFileSync|createWriteStream)", 0, "write"),
        Call(r"(?:fs\.|fsp\.)?(?:mkdirSync|mkdir)", 0, "create"),
        Call(r"(?:fs\.|fsp\.)?(?:copyFileSync|copyFile|cpSync)", 1, "copy"),
        Call(r"(?:fs\.|fsp\.)?renameSync", 1, "move"),
    ),
}
QUOTES = {"py": "'\"", "js": "'\"`"}
CALL_SITES = {lang: [(c, re.compile(rf"(?<!\w)(?:{c.callee})\s*\(")) for c in calls] for lang, calls in CALLS.items()}


def call_args(code, i, quotes):
    """Arguments of the call whose `(` ends just before i, and the index after its `)`."""
    args, start, depth, quote = [], i, 0, None
    while i < len(code):
        ch = code[i]
        if quote:
            i += 2 if ch == "\\" else 1
            quote = None if ch == quote else quote
            continue
        if ch in quotes:
            quote = ch
        elif ch in "([{":
            depth += 1
        elif ch in ")]}":
            if depth == 0:
                args.append(code[start:i].strip())
                return args, i + 1
            depth -= 1
        elif ch == "," and depth == 0:
            args.append(code[start:i].strip()); start = i + 1
        i += 1
    return None, i  # unterminated: a truncated script


def literal(arg, lang):
    """The value of an argument that is a single plain string literal, else None."""
    m = re.fullmatch(r"([rRuU]?)(['\"`])(.*)\2", arg, re.S) if lang == "py" or arg[:1] in QUOTES[lang] else None
    if not m or (lang == "py" and m[2] == "`") or (m[2] in m[3].replace("\\" + m[2], "")):
        return None
    text = m[3]
    if "{" in text or "}" in text or "\n" in text:
        return None  # a template or a format string
    return text if m[1] in ("r", "R") else text.replace("\\\\", "\\")


def code_ops(code, lang):
    found = []
    for call, site in CALL_SITES[lang]:
        for m in site.finditer(code):
            args, end = call_args(code, m.end(), QUOTES[lang])
            if not args or len(args) <= call.arg:
                continue
            if call.then and not re.match(call.then, code[end:]):
                continue
            if call.mode is not None:
                mode = literal(re.sub(r"^mode\s*=\s*", "", args[call.mode]), lang) if len(args) > call.mode else None
                if not mode or not set(mode) & set("wax"):
                    continue
            path = literal(args[call.arg], lang)
            if path:
                found.append((m.start(), op(call.kind, dst=path)))
    return [o for _, o in sorted(found, key=lambda x: x[0])]


def script_source(a, ctx, inline_flags):
    """The source an interpreter runs: an inline -c/-e string, a heredoc, or a file the same command
    wrote from a heredoc. None when the source is somewhere else."""
    w = words(a)
    for i, x in enumerate(w):
        if x in inline_flags and i + 1 < len(w):
            return w[i + 1]
    here = next((t for t in a if isinstance(t, Op) and t == "<<"), None)
    if here is not None:
        return here.body
    return next((ctx["scripts"][x] for x in w if x in ctx["scripts"]), None)


def f_python(a, sh, flags, ctx):
    w = words(a)
    if "-m" in w and w[w.index("-m") + 1:w.index("-m") + 2] in (["venv"], ["virtualenv"]):
        pos, _ = posix(w[w.index("-m") + 2:], {"--prompt"})
        return [op("create", dst=p) for p in pos]
    code = script_source(a, ctx, ("-c",))
    if code is not None:
        flags.add("script")
        return code_ops(code, "py")
    return []


def f_eval(a, sh, flags, ctx):  # node/deno/bun -e, a heredoc, or a script the command just wrote
    code = script_source(a, ctx, ("-e", "--eval", "-p", "--print"))
    if code is not None:
        flags.add("script")
        return code_ops(code, "js")
    return []


INTERPRETERS = {"python", "python3", "py", "node", "deno", "bun", "ruby", "perl", "php", "bash", "sh", "zsh",
                "pwsh", "powershell"}


def f_sed(a, sh):  # sed -i edits its file arguments in place
    w = words(a)
    if not any(x == "--in-place" or x.startswith("--in-place=") or re.fullmatch(r"-[nErsz]*i\S*", x) for x in w):
        return []
    pos, o = posix([x for x in w if not re.fullmatch(r"-[nErsz]*i\S*", x)], {"-e", "--expression", "-f", "--file"})
    files = pos if ("-e" in o or "--expression" in o or "-f" in o or "--file" in o) else pos[1:]
    return [op("write", dst=f) for f in files]


def f_dotnet(a, sh):
    w = words(a)
    if w[:1] == ["new"]:
        _, o = posix(w[1:], {"-o", "--output", "-n", "--name", "-f", "--framework", "-lang", "--language"})
        dst = o.get("-o") or o.get("--output") or o.get("-n") or o.get("--name")
        return [op("create", dst=dst)] if isinstance(dst, str) else []
    return []


def f_tar(a, sh):
    w, mode, archive, dst, i = words(a), "", None, None, 0
    while i < len(w):
        t = w[i]
        if t.startswith("--"):
            k, eq, v = t.partition("=")
            if k in ("--file", "--directory"):
                if not eq:
                    v = w[i + 1] if i + 1 < len(w) else None; i += 1
                archive, dst = (v, dst) if k == "--file" else (archive, v)
            mode += {"--extract": "x", "--get": "x", "--create": "c"}.get(k, "")
        elif t.startswith("-") or (i == 0 and t.isalpha()):
            letters = t.lstrip("-")
            for j, ch in enumerate(letters):
                if ch in "fC":
                    v = letters[j + 1:] or (w[i + 1] if i + 1 < len(w) else None)
                    i += 0 if letters[j + 1:] else 1
                    archive, dst = (v, dst) if ch == "f" else (archive, v)
                    break
                mode += ch
        i += 1
    if "x" in mode:
        return [op("copy", src=archive, dst=dst)]
    return [op("write", dst=archive)] if "c" in mode and archive not in (None, "-") else []


def f_unzip(a, sh):
    pos, o = posix(a, {"-d", "-P"})
    return [op("copy", src=pos[0], dst=o.get("-d"))] if pos else []


def f_expand_archive(a, sh):
    pos, o = ps(a, {**PATH, "destinationpath": "dst"})
    src = (o.get("path") or (pos.pop(0) if pos else [None]))[0]
    dst = (o.get("dst") or (pos[0] if pos else [None]))[0]
    return [op("copy", src=src, dst=dst)] if src else []


def f_file_writer(a, sh):  # tee, Tee-Object, Out-File, Set-Content, Add-Content, touch
    if sh == "powershell":
        pos, o = ps(a, FILE_PARAMS)
        return [op("write", dst=p) for p in (o.get("path") or (pos[0] if pos else []))]
    return [op("write", dst=p) for p in posix(a, {"-d", "-r", "-t"})[0]]


def f_nested(a, sh, flags, name):
    w = words(a)
    if name in ("bash", "sh", "zsh", "dash"):
        i = next((i for i, x in enumerate(w) if re.fullmatch(r"-[a-z]*c[a-z]*", x)), None)
        script, inner = (w[i + 1] if i is not None and i + 1 < len(w) else None), "bash"
    elif name in ("powershell", "pwsh"):
        i = next((i for i, x in enumerate(w) if x.lower() in ("-c", "-command", "-com", "-comm")), None)
        script, inner = (" ".join(w[i + 1:]) if i is not None else None), "powershell"
        if any(x.lower() in ("-file", "-f", "-encodedcommand", "-e", "-ec") for x in w):
            flags.add("script")
    else:
        i = next((i for i, x in enumerate(w) if x.lower() in ("/c", "/k")), None)
        script, inner = (" ".join(w[i + 1:]) if i is not None else None), "cmd"
    if not script:
        return []
    ops, inner_flags = extract(script, inner)
    flags.update(inner_flags)
    return [op("cd", dst=".", push=True)] + ops + [op("cd", pop=True)]


def command(w, sh, flags, ctx):
    while w and (str(w[0]).lower() in PREFIXES or ASSIGNMENT.match(w[0])):
        w = w[1:]
    if not w or isinstance(w[0], Op):
        return []
    name = re.split(r"[\\/]", w[0])[-1].lower().removesuffix(".exe")
    a = w[1:]
    if name in LOOPS:
        flags.add("loop")
        return []
    table = {
        "cd": f_cd, "chdir": f_cd, "set-location": f_cd, "sl": f_cd,
        "pushd": lambda a, sh: f_cd(a, sh, push=True), "push-location": lambda a, sh: f_cd(a, sh, push=True),
        "popd": lambda a, sh: [op("cd", pop=True)], "pop-location": lambda a, sh: [op("cd", pop=True)],
        "mkdir": f_mkdir, "md": f_mkdir,
        "new-item": f_new_item, "ni": f_new_item,
        "cp": lambda a, sh: f_copy(a, sh, "copy"), "copy": lambda a, sh: f_copy(a, sh, "copy"),
        "copy-item": lambda a, sh: f_copy(a, "powershell", "copy"), "cpi": lambda a, sh: f_copy(a, sh, "copy"),
        "mv": lambda a, sh: f_copy(a, sh, "move"), "move": lambda a, sh: f_copy(a, sh, "move"),
        "move-item": lambda a, sh: f_copy(a, "powershell", "move"), "mi": lambda a, sh: f_copy(a, sh, "move"),
        "robocopy": f_robocopy, "xcopy": f_robocopy,
        "git": f_git, "cargo": f_cargo,
        "npm": f_npm, "pnpm": f_npm, "yarn": f_npm, "bun": f_npm, "npx": f_npx, "pnpx": f_npx, "bunx": f_npx,
        "uv": f_uv, "dotnet": f_dotnet,
        "virtualenv": lambda a, sh: [op("create", dst=p) for p in posix(a, {"-p", "--python", "--prompt"})[0]],
        "tar": f_tar, "bsdtar": f_tar, "unzip": f_unzip, "expand-archive": f_expand_archive,
        "tee": f_file_writer, "tee-object": f_file_writer, "out-file": f_file_writer, "set-content": f_file_writer,
        "add-content": f_file_writer, "sc": f_file_writer, "ac": f_file_writer, "touch": f_file_writer,
    }
    if name in INTERPRETERS and "<<" in a:  # a script read from a heredoc: Xiox does not interpret it
        flags.add("script")
    if name in ("python", "python3", "py"):
        ops = f_python(a, sh, flags, ctx)
    elif name == "sed":
        ops = f_sed(a, sh)
    elif name in ("node", "deno", "bun") and not (name == "bun" and words(a)[:1] in (["create"], ["init"])):
        ops = f_eval(a, sh, flags, ctx)
    elif name in ("bash", "sh", "zsh", "dash", "powershell", "pwsh", "cmd"):
        ops = f_nested(a, sh, flags, name)
    elif name in table:
        ops = table[name](a, sh)
    else:
        ops = []
    if not ops:  # an output directory named by a tool flag
        ws = words(a)
        for i, x in enumerate(ws):
            k, eq, v = x.partition("=")
            if k in OUTPUT_FLAGS:
                v = v if eq else (ws[i + 1] if i + 1 < len(ws) else None)
                ops += [op("write", dst=v)] if v else []
    return ops


VAR_REF = {"bash": re.compile(r"\$\{(\w+)\}|\$(\w+)"), "powershell": re.compile(r"\$\{?([A-Za-z_]\w*)\}?()"),
           "cmd": re.compile(r"%(\w+)%()")}


def assignment(w, shell):
    """(name, value) of a segment that only assigns a literal, else None."""
    ws = [x for x in w if not isinstance(x, Op)]
    if shell == "bash" and len(ws) == 1 and ASSIGNMENT.match(ws[0]):
        name, value = ws[0].split("=", 1)
    elif shell == "powershell" and len(ws) == 3 and ws[1] == "=" and re.fullmatch(r"\$\w+", ws[0]):
        name, value = ws[0][1:].lower(), ws[2]
    elif shell == "powershell" and len(ws) == 1 and re.fullmatch(r"\$\w+=.+", ws[0]):
        name, value = ws[0][1:].split("=", 1)
        name = name.lower()
    elif shell == "cmd" and len(ws) == 2 and ws[0].lower() == "set" and "=" in ws[1]:
        name, value = ws[1].split("=", 1)
        name = name.lower()
    else:
        return None
    return name, value


def substitute(w, shell, known):
    """Replace references to literally assigned variables; unknown ones stay and resolve to nothing."""
    def one(x):
        if isinstance(x, Op):
            return x
        def rep(m):
            name = m[1] or m[2]
            name = name if shell == "bash" else name.lower()
            return known.get(name, m[0])
        return VAR_REF[shell].sub(rep, x)
    return [one(x) for x in w]


LITERAL_ITEM = re.compile(r"[^$`*?\[\]{}()]+")
LOOP_HEADS = {"for", "while", "until", "select"}


def heads(w):
    ws = [x for x in w if not isinstance(x, Op)]
    while ws and ws[0] in ("do", "then", "else", "{"):
        ws = ws[1:]
    return ws


def run(segs, shell, flags, known, ctx):
    ops, i = [], 0
    while i < len(segs):
        w, redirs = segs[i]
        head = heads(w)
        if shell == "bash" and head[:1] == ["for"]:  # for V in a b c; do ...; done: literal lists are expanded
            j, depth = i + 1, 1
            while j < len(segs):
                h = heads(segs[j][0])[:1]
                depth += 1 if h and h[0] in LOOP_HEADS else -1 if h == ["done"] else 0
                if depth == 0:
                    break
                j += 1
            body = segs[i + 1:j]
            items = substitute(head[3:], shell, known) if len(head) > 2 and head[2] == "in" else []
            if items and len(items) <= 50 and all(LITERAL_ITEM.fullmatch(x) for x in items):
                for v in items:
                    ops += run(body, shell, flags, {**known, head[1]: v}, ctx)
            else:
                flags.add("loop")
                ops += run(body, shell, flags, dict(known), ctx)
            if j < len(segs):  # done > file
                ops += [op("write", dst=r) for r in substitute(segs[j][1], shell, known)]
            i = j + 1
            continue
        a = assignment(w, shell)
        if a:
            value = substitute([a[1]], shell, known)[0]
            if not UNKNOWN.search(value):  # only a literal value is ever substituted
                known[a[0]] = value
            else:
                known.pop(a[0], None)
            i += 1
            continue
        if known:
            w, redirs = substitute(w, shell, known), substitute(redirs, shell, known)
        here = next((t for t in w if isinstance(t, Op) and t == "<<" and t.body is not None), None)
        if here is not None and head[:1] in (["cat"], ["tee"]) and redirs + head[1:]:
            for target in (redirs or head[1:]):  # cat > x.py << EOF: the body may run later in the command
                ctx["scripts"][target] = here.body
        ops += command(w, shell, flags, ctx)
        ops += [op("write", dst=r) for r in redirs]
        i += 1
    return ops


def extract(cmd, shell):
    """(raw operations in order, uncertainty flags: loop, script)."""
    flags = set()
    ops = run(segments(scan(cmd, shell)), shell, flags, {}, {"scripts": {}})
    return ops, sorted(flags)


# ---- resolver, shared by both extractors

NULL_DEVICES = {"/dev/null", "nul", "nul:", "$null", "-"}
URL = re.compile(r"^[a-z][a-z0-9+.-]*://|^[\w.-]+@[\w.-]+:", re.I)
UNKNOWN = re.compile(r"[$`*?]|%[^%\s]+%")


class Unresolved(Exception):
    pass


def resolve_path(p, cur, env):
    p = str(p).strip()
    if UNKNOWN.search(p):
        raise Unresolved(p)
    if p == "~" or p.startswith(("~/", "~\\")):
        p = env["home"] + p[1:]
    m = re.match(r"^/([a-zA-Z])(?:/|$)(.*)", p)
    if m:
        p = f"{m[1].upper()}:\\{m[2]}"
    elif p == "/tmp" or p.startswith("/tmp/"):
        p = env["temp"] + p[4:]
    elif p.startswith("/") and not p.startswith("//"):
        raise Unresolved(p)  # another MSYS root
    p = p.replace("/", "\\")
    if not re.match(r"^[a-zA-Z]:\\|^\\\\", p):
        if cur is None or re.match(r"^[a-zA-Z]:|^\\", p):
            raise Unresolved(p)  # unknown cwd, drive-relative or root-relative
        p = ntpath.join(cur, p)
    p = ntpath.normpath(p)
    return p.rstrip("\\") if len(p) > 3 else p


def repo_name(src):
    name = re.split(r"[/\\:]", src.rstrip("/\\"))[-1]
    return name.removesuffix(".git") or None


def resolve(ops, cwd, env):
    """(candidates [{op, src, dst}], count of operations whose path could not be resolved)."""
    cur, stack, out, bad = cwd, [], [], 0
    for o in ops:
        kind, src, dst = o.get("op"), o.get("src"), o.get("dst")
        if kind == "cd":
            if o.get("pop"):
                cur = stack.pop() if stack else cur
                continue
            if o.get("push"):
                stack.append(cur)
            if dst is not None:
                try:
                    cur = resolve_path(dst, cur, env)
                except Unresolved:
                    cur, bad = None, bad + 1
            continue
        if kind not in CANDIDATE_OPS or (dst is not None and str(dst).strip().lower() in NULL_DEVICES):
            continue
        try:
            base = resolve_path(o["base"], cur, env) if o.get("base") else cur
            if src is not None and not URL.match(src):
                try:
                    src = resolve_path(src, base, env)
                except Unresolved:
                    src = None
            if dst is None:
                dst = repo_name(o["src"]) if kind == "clone" and o.get("src") else "." if kind == "copy" else None
            if dst is None:
                continue
            out.append({"op": kind, "src": src, "dst": resolve_path(dst, base, env)})
        except Unresolved:
            bad += 1
    return out, bad
