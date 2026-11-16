"""The benchmark on real sessions: shell calls collected by harvest.py.

    python real.py sample --calls calls.jsonl --out sample.json [--per-stratum 60] [--seed 7]
        deterministic extraction on every shell call; a stratified sample for the model and labels
    python real.py model --sample sample.json --server <llama-server.exe> --model <gguf> [--threads 4]
        the model on the sample (same prompt, schema and guard as bench.py)
    python real.py score --sample sample.json --labels labels.json
        labels.json: {event id: [[op, resolved dst], ...]}, written by hand from the sample
    python real.py parity --calls calls.jsonl --rust <dir> [--home <dir>]
        the Rust probe's output (crates/xiox-project/examples/shell_probe.rs) against this prototype,
        field by field; declared divergences D2-D13 are counted apart. --self-test needs neither.

Strata: det-found (the deterministic extractor gives a candidate), det-unsure (it flags a loop or
script, or a path it cannot resolve), det-empty (neither). Real commands stay in the scratch files."""
import argparse
import collections
import json
import ntpath
import os
import random
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bench import key, run_model, score, table  # noqa: E402
from extract import extract, resolve  # noqa: E402

SHELLS = {"bash": "bash", "power_shell": "powershell", "powershell": "powershell", "shell": None,
          "exec_command": None, "terminal": "bash", "cmd": "cmd"}
# The home of the machine the sessions ran on; XIOX_BENCH_HOME when the reading shell has another one.
HOME = os.environ.get("XIOX_BENCH_HOME") or os.path.expanduser("~")
ENV = {"home": HOME, "temp": os.path.join(HOME, "AppData", "Local", "Temp")}


def shell_of(c):
    """(shell, command) of a shell call, or None."""
    canon = c["canonical"] if isinstance(c["canonical"], str) else None
    tool = (c["tool"] or "").lower()
    inp = c["input"]
    if canon not in SHELLS and tool not in SHELLS:
        return None
    cmd = inp.get("command", inp.get("cmd"))
    if isinstance(cmd, list):  # argv form: ["powershell.exe", "-Command", "..."] or ["bash", "-lc", "..."]
        cmd = " ".join(f'"{a}"' if " " in a else a for a in cmd)
    if not isinstance(cmd, str) or not cmd.strip():
        return None
    sh = SHELLS.get(canon) or SHELLS.get(tool)
    if sh is None:
        hint = str(inp.get("shell") or inp.get("shell_type") or "").lower()
        sh = "powershell" if "power" in hint or "pwsh" in hint else "cmd" if hint == "cmd" else "bash"
    return sh, cmd


def load_calls(path):
    events, census = [], collections.Counter()
    for i, line in enumerate(open(path, encoding="utf-8")):
        c = json.loads(line)
        s = shell_of(c)
        census[(c["harness"], c["tool"], s[0] if s else None)] += 1
        if s and c.get("cwd"):
            cwd = c["cwd"].removeprefix("\\\\?\\").replace("/", "\\")
            events.append({"id": f"e{i}", "harness": c["harness"], "session": c["session"], "entry": c["entry"],
                           "shell": s[0], "cwd": cwd, "cmd": s[1].strip(), "error": c.get("error"),
                           "exit": c.get("exit"), "cat": None})
    return events, census


def cmd_sample(args):
    events, census = load_calls(args.calls)
    print("shell calls by harness/tool:")
    for (h, tool, sh), n in sorted(census.items(), key=lambda x: -x[1]):
        if sh:
            print(f"  {n:>7}  {h:<17} {tool:<22} -> {sh}")
    strata = collections.defaultdict(list)
    ops = collections.Counter()
    crashes = collections.Counter()
    for ev in events:
        try:
            raw, ev["det_flags"] = extract(ev["cmd"], ev["shell"])
            ev["det_c"], ev["det_unresolved"] = resolve(raw, ev["cwd"], ENV)
        except Exception as e:  # counted: the extractor must never stop on a real command
            crashes[f"{type(e).__name__}: {e}"[:100]] += 1
            ev["det_c"], ev["det_flags"], ev["det_unresolved"], ev["det_crash"] = [], ["crash"], 0, repr(e)
        ops.update(c["op"] for c in ev["det_c"])
        ev["cat"] = "det-found" if ev["det_c"] else "det-unsure" if ev["det_flags"] or ev["det_unresolved"] else "det-empty"
        strata[ev["cat"]].append(ev)
    by_h = collections.Counter((ev["harness"], ev["cat"]) for ev in events)
    print(f"\n{len(events)} shell events with a cwd")
    for h in sorted({ev['harness'] for ev in events}):
        print(f"  {h:<17} " + "  ".join(f"{s} {by_h[(h, s)]}" for s in ("det-found", "det-unsure", "det-empty")))
    print("deterministic candidates by op:", dict(ops))
    print("extractor crashes:", sum(crashes.values()), dict(crashes.most_common(8)))
    rnd = random.Random(args.seed)
    used = set()
    for path in args.exclude or []:  # a held-out sample never repeats the events used to build the rules
        used |= {(e["session"], e["entry"], e["cmd"]) for e in json.loads(Path(path).read_text(encoding="utf-8"))["events"]}
    for s in strata:
        strata[s] = [ev for ev in strata[s] if (ev["session"], ev["entry"], ev["cmd"]) not in used]
    sample = [ev for s in ("det-found", "det-unsure", "det-empty")
              for ev in rnd.sample(strata[s], min(args.per_stratum, len(strata[s])))]
    Path(args.out).write_text(json.dumps({"population": {s: len(v) for s, v in strata.items()},
                                          "events": sample}, indent=1, ensure_ascii=False), encoding="utf-8")
    print(f"sample: {len(sample)} events -> {args.out}")


def cmd_model(args):
    data = json.loads(Path(args.sample).read_text(encoding="utf-8"))
    args.out = args.sample  # the server log goes beside the sample
    info = run_model(data["events"], ENV, args)
    data["model_info"] = info
    Path(args.sample).write_text(json.dumps(data, indent=1, ensure_ascii=False), encoding="utf-8")
    print(json.dumps(info))


def cmd_score(args):
    data = json.loads(Path(args.sample).read_text(encoding="utf-8"))
    labels = json.loads(Path(args.labels).read_text(encoding="utf-8"))
    evs = [ev for ev in data["events"] if ev["id"] in labels]
    for ev in evs:
        ev["gold_c"] = [{"op": o, "src": None, "dst": d} for o, d in labels[ev["id"]]]
        raw, _ = extract(ev["cmd"], ev["shell"])  # the extractor as it is now, beside the sampled one
        ev["det_now_c"], _ = resolve(raw, ev["cwd"], ENV)
        unsure = ev["det_flags"] or ev["det_unresolved"]
        if "guard_c" in ev:
            ev["hybrid_c"] = ev["det_c"] + [c for c in ev["guard_c"] if unsure and key(c) not in {key(d) for d in ev["det_c"]}]
    fields = [("deterministic", "det_c"), ("deterministic now", "det_now_c")] + ([("model", "model_c"), ("model+span guard", "guard_c"),
                                              ("hybrid", "hybrid_c")] if "guard_c" in evs[0] else [])
    print(f"labelled {len(evs)} of {len(data['events'])}; population {data['population']}")
    for name, field in fields:
        total, rows = score(evs, field)
        table(name, total)
        for s, r in rows.items():
            print(f"    {s:<11} exact {r['exact']}/{r['events']}  correct {r['correct']}  invented {r['invented']}  missed {r['missed']}")


# ---- parity with the Rust extractor and ledger (crates/xiox-project/examples/shell_probe.rs)

COMPARED_FLAGS = {"loop": "looped", "script": "script"}   # prototype flag -> probe flag; the rest are classes
HEREDOC = re.compile(r"<<-?\s*(['\"]?)([A-Za-z_]\w*)\1")
BUN_INIT = re.compile(r"\bbun\s+init\b")
NESTED_SHELL = re.compile(r"\b(?:ba|z|da)?sh\b[^\n]*\s-\w*c\b|\b(?:pwsh|powershell)(?:\.exe)?\b[^\n]*-Command", re.I)
FILE_WRITES = re.compile(r"\bcat\s*>>?\s*(\S+)|\btee\s+(?:-a\s+)?(\S+)|<<[^\n]*?>\s*(\S+)")
CLASS_ORDER = ("D2", "D3", "D4", "D5/D6", "D7", "D8", "D9", "D10", "D11", "D12", "D13")
# `cd -`, `cd -P dir`, `cd -- dir`, PowerShell `cd -`: the prototype takes the option word as the directory.
D10_CD = re.compile(r"(?i)(?:^|[;&|(\n])\s*(?:cd|chdir|pushd|push-location|set-location|sl)\s+(?:-[LPe@]*|--)(?=\s|$|[;&|)])")
INVENTED_DIR = re.compile(r"\\-[^\\]*(?:\\|$)")   # a path component that is an option word
BRACE_EXPR = re.compile(r"\{[^{}\s]*(?:,|\.\.)[^{}\s]*\}")

# A declared divergence explains a difference only in the directions it can make the Rust side differ
# (+1: Rust has more, -1: fewer, 0: a different set) and only in the fields it touches. D6 (script
# arguments counted in Rust only) touches no compared field: it is counted, never an explanation.
FEWER, MORE, SAME = -1, 1, 0
EXPLAINS = {"D2": ((MORE,), {"candidates", "flags", "unresolved"}),    # Python swallows the commands after `EOF\r`
            "D3": ((FEWER,), {"candidates", "flags", "unresolved"}),   # the cut text drops its last command
            "D4": ((FEWER,), {"candidates", "flags", "unresolved"}),   # the budget ran out
            "D5/D6": ((FEWER,), {"candidates"}),                       # Python also finds callers inside strings
            "D8": ((MORE,), {"candidates"}),                           # `bun init <dir>` creates the dir
            "D9": ((MORE,), {"candidates"}),                           # a nested shell reads the outer script
            "D10": ((FEWER, SAME), {"candidates", "unresolved"}),      # the option word was a directory
            "D11": ((FEWER, MORE, SAME), {"candidates", "flags", "unresolved"}),  # a brace expression was one literal path
            "D12": ((FEWER,), {"candidates", "unresolved"}),           # a path Windows cannot hold was a candidate
            "D13": ((FEWER, MORE, SAME), {"candidates", "unresolved"})}      # here-string text was words and paths
# The directions a class explains in the other fields and in the folders, when they are not those above.
FIELD_WAYS = {("D10", "unresolved"): (MORE,), ("D11", "unresolved"): (MORE, FEWER), ("D11", "flags"): (FEWER,),
              ("D12", "unresolved"): (MORE,), ("D13", "unresolved"): (FEWER,)}
FOLDER_WAYS = {"D10": (FEWER, MORE), "D11": (FEWER, MORE), "D12": (FEWER,), "D13": (FEWER,)}
# `for x in a b{1,2}`: the items hold a brace expression, which Rust expands and the prototype leaves unresolved.
FOR_BRACE = re.compile(r"\bfor\s+\w+\s+in\s+([^;\n]*\{[^{}\s]*(?:,|\.\.)[^{}\s]*\}[^;\n]*)")
# `@'` or `@"` ending its line, at the start of a word: a PowerShell here-string.
HERE_OPEN = re.compile(r"(?:^|(?<=[\s;|&(,{}]))@(['\"])\r?\n")
IMPOSSIBLE = re.compile(r'[\x00-\x1f\x7f"<>|]')   # characters a Windows path cannot hold, besides a colon past the drive


def impossible(path):
    """D12: whether a path holds a character Windows forbids (a colon is fine only as the drive's)."""
    return bool(IMPOSSIBLE.search(path) or ":" in re.sub(r"^[A-Za-z]:", "", path))


def here_strings(text):
    """D13: the bodies of the here-strings in a PowerShell text; an unterminated one runs to the end."""
    bodies, at = [], 0
    while (m := HERE_OPEN.search(text, at)):
        close = re.compile(r"^" + m[1] + "@", re.M).search(text, m.end())
        bodies.append(text[m.end():close.start() if close else len(text)])
        at = close.end() if close else len(text)
    return bodies


def crlf_tail(text):
    """D2: the text after the first heredoc delimiter followed by a carriage return, else None."""
    for m in HEREDOC.finditer(text):
        end = re.compile(rf"(?m)^[ \t]*{re.escape(m[2])}\r$").search(text, m.end())
        if end:
            return text[end.end():]
    return None


def written_scripts(text):
    """D9: the bodies of the heredocs before a nested shell whose command names a file that a heredoc,
    `cat >` or `tee` wrote; empty when there is no such shell."""
    nested = NESTED_SHELL.search(text)
    if not nested:
        return []
    head = text[:nested.start()]
    named = False
    for w in FILE_WRITES.finditer(head):
        name = ntpath.basename(next(g for g in w.groups() if g).strip("'\"").replace("/", "\\"))
        named = named or bool(name and name in text[nested.start():])
    if not named:
        return []
    bodies = []
    for m in HEREDOC.finditer(head):
        start = head.find("\n", m.end()) + 1
        end = re.compile(rf"(?m)^[ \t]*{re.escape(m[2])}[ \t\r]*$").search(head, start)
        bodies.append(head[start:end.start() if end else len(head)])
    return bodies


def brace_expand(word):
    """Bash brace expansion of one word, written apart from the Rust one: lists, nesting, sequences."""
    if word is None:
        return [None]
    at = word.find("{")
    while at >= 0:
        depth, commas, close = 0, [], -1
        for i in range(at, len(word)):
            if word[i] == "{":
                depth += 1
            elif word[i] == "}":
                depth -= 1
                if depth == 0:
                    close = i
                    break
            elif word[i] == "," and depth == 1:
                commas.append(i)
        if close < 0:
            return [word]
        body = word[at + 1:close]
        edges = [at] + commas + [close]
        items = ([x for a, b in zip(edges, edges[1:]) for x in brace_expand(word[a + 1:b])] if commas
                 else sequence_of(body))
        if items:
            return [word[:at] + x + t for x in items for t in brace_expand(word[close + 1:])]
        at = word.find("{", at + 1)
    return [word]


def sequence_of(body):
    m = re.fullmatch(r"(-?\d+)\.\.(-?\d+)(?:\.\.(-?\d+))?", body)
    step = abs(int(m[3])) if m and m[3] else 1
    if m and step:
        a, b = int(m[1]), int(m[2])
        width = max((len(x.lstrip("-")) for x in (m[1], m[2]) if len(x.lstrip("-")) > 1 and x.lstrip("-")[0] == "0"), default=0)
        values = range(a, b + 1, step) if b >= a else range(a, b - 1, -step)
        return [("-" if v < 0 else "") + str(abs(v)).zfill(width) for v in values]
    m = re.fullmatch(r"([A-Za-z])\.\.([A-Za-z])(?:\.\.(-?\d+))?", body)
    step = abs(int(m[3])) if m and m[3] else 1
    if m and step:
        a, b = ord(m[1]), ord(m[2])
        return [chr(v) for v in (range(a, b + 1, step) if b >= a else range(a, b - 1, -step))]
    return []


def counts(items):
    return collections.Counter(tuple(x) if isinstance(x, list) else x for x in items)


def direction(py, rust):
    """FEWER or MORE when one side is a proper part of the other; SAME for a different order or set."""
    if isinstance(py, int):
        return (rust > py) - (rust < py)
    a, b = counts(py), counts(rust)
    return FEWER if b < a else MORE if b > a else SAME


class Diff:
    """One differing field of a call, with what each class needs to judge it."""

    def __init__(self, call, field, py, rust, tail_info):
        self.call, self.field, self.py, self.rust, self.tail_info = call, field, py, rust, tail_info
        self.sign = direction(py, rust)
        if field == "candidates":
            self.rust_only = list((counts(rust) - counts(py)).elements())
            self.py_only = list((counts(py) - counts(rust)).elements())
        elif field == "flags":
            self.rust_only = sorted(set(rust) - set(py))


def explains(cls, diff):
    ways, fields = EXPLAINS[cls]
    ways = FIELD_WAYS.get((cls, diff.field), ways)
    if diff.sign not in ways or diff.field not in fields:
        return False
    call = diff.call
    if cls == "D2":   # what Rust adds must come from the swallowed tail
        if diff.field == "candidates":
            return all(ntpath.basename(c[2]) in call["tail"] for c in diff.rust_only)
        if diff.field == "flags":
            return set(diff.rust_only) <= diff.tail_info["flags"]
        return diff.rust - diff.py <= diff.tail_info["unresolved"]
    if diff.field == "candidates":
        if cls == "D8":   # only a create with no source
            return all(c[0] == "create" and c[1] is None for c in diff.rust_only)
        if cls == "D9":   # what Rust adds must be written in a script an earlier heredoc made
            bodies = [b.replace("/", "\\").lower() for b in call["scripts"]]
            cwd = call["cwd"].lower() + "\\"

            def written(c):
                dst = c[2].lower()
                return any(dst in b or dst.removeprefix(cwd) in b for b in bodies)
            return all(written(c) for c in diff.rust_only)
        if cls == "D10":   # what Rust lacks sits under a directory named like an option, and Rust adds no more
            return (bool(diff.py_only) and all(INVENTED_DIR.search(c[2]) for c in diff.py_only)
                    and len(diff.rust_only) <= len(diff.py_only))
        if cls == "D11":
            if not diff.py_only:   # a loop over brace items: Rust expands them, what it adds names an item
                return bool(diff.rust_only) and call["loop"] and all(names_an_item(c, call) for c in diff.rust_only)
            # what Rust lacks has a brace expression, and Rust has only its expansion
            if not all(BRACE_EXPR.search(c[2]) or BRACE_EXPR.search(c[1] or "") for c in diff.py_only):
                return False
            expected = {(c[0], s, d) for c in diff.py_only for s in brace_expand(c[1]) for d in brace_expand(c[2])}
            return all(tuple(c) in expected for c in diff.rust_only)
        if cls == "D12":   # what Rust lacks holds a character Windows forbids, and Rust adds nothing
            return bool(diff.py_only) and not diff.rust_only and all(impossible(c[2]) for c in diff.py_only)
        if cls == "D13":   # what Python adds comes from the here-string's text; what Rust adds is written outside it
            return bool(call["here"]) and all(in_here_string(c, call) for c in diff.py_only) \
                and all(written_outside(c, call) for c in diff.rust_only)
    if cls == "D11" and diff.field == "flags":   # Rust has no `looped` where the loop items were braces
        return call["loop"]
    if diff.field == "unresolved":
        if cls == "D10":
            return call["cand_dir"] != MORE
        if cls == "D11":   # more: the limit left a word unknown; fewer: the loop's variable was expanded
            return call["cand_dir"] == FEWER if diff.sign == MORE else call["loop"]
        if cls == "D12":
            return call["cand_dir"] != MORE
    return True


def names_an_item(cand, call):
    """D11: whether a path of the loop body holds one of the expanded `for` items as a component start."""
    items = {i.replace("/", "\\").lower() for w in call["loop_items"].split() for i in brace_expand(w)}
    return any("\\" + i in cand[2].lower() for i in items)


def in_here_string(cand, call):
    """D13: whether a candidate the prototype made is built from here-string text."""
    rel = cand[2][len(call["cwd"]) + 1:] if cand[2].lower().startswith(call["cwd"].lower() + "\\") else cand[2]
    words = {w for body in call["here"] for w in body.split() if len(w) > 1}
    return "@" in rel or any(w in rel or w.replace("/", "\\") in rel for w in words)


def written_outside(cand, call):
    """D13: whether a candidate Rust made is spelled in the command outside its here-strings."""
    outside = call["text"]
    for body in call["here"]:
        outside = outside.replace(body, "")
    outside = outside.replace("/", "\\").lower()
    dst = cand[2].lower()
    return dst in outside or dst.removeprefix(call["cwd"].lower() + "\\") in outside


def read_rows(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def keyed(rows, key):
    """Rows by key; a key met again (a call with no ids) gets its occurrence number appended."""
    seen, out = collections.Counter(), {}
    for row in rows:
        k = key(row)
        out[k + (seen[k],) if seen[k] else k] = row
        seen[k] += 1
    return out


def call_key(row):
    return (row["store"], row["session"], row["entry"], row["call"])


def folder_key(row):
    return (row["store"], row["session"], row["folder"].lower())


def tail_extraction(tail, row):
    """D2: the flags and unresolved count the prototype finds in the text it swallowed."""
    raw, flags = extract(tail.strip(), row["shell"])
    return {"flags": {COMPARED_FLAGS[f] for f in flags if f in COMPARED_FLAGS},
            "unresolved": resolve(raw, row["cwd"], ENV)[1]}


def python_calls(calls_path):
    """The prototype's rows for the shell calls that have a cwd, in the probe's shape."""
    rows = []
    for c in read_rows(calls_path):
        s = shell_of(c)
        if not (s and c.get("cwd")):
            continue
        cwd = c["cwd"].removeprefix("\\?\\").replace("/", "\\")
        raw, flags = extract(s[1].strip(), s[0])
        cands, unresolved = resolve(raw, cwd, ENV)
        rows.append({"store": c.get("store"), "session": c.get("session"), "entry": c.get("entry"), "call": c.get("call"),
                     "candidates": [[o["op"], o["src"], o["dst"]] for o in cands],
                     "flags": sorted(COMPARED_FLAGS[f] for f in flags if f in COMPARED_FLAGS),
                     "unresolved": unresolved, "text": s[1], "shell": s[0], "cwd": cwd})
    return rows


class Parity:
    """Differences by field (ids only) and the declared-divergence classes, for calls and for folders."""

    def __init__(self):
        self.diffs = collections.defaultdict(list)
        self.classes = {kind: collections.defaultdict(list) for kind in ("calls", "folders")}
        self.compared = collections.Counter()
        self.loops = set()   # calls whose brace-item `for` loop D11 explained: Rust's folders from it are D11 too

    def total(self):
        return sum(len(v) for v in self.diffs.values())

    def print(self, out=sys.stdout):
        for kind, by_class in self.classes.items():
            parts = [f"{c} {len(by_class[c])}" for c in CLASS_ORDER if by_class[c]]
            print(f"declared divergences, {kind}: " + (", ".join(parts) or "none"), file=out)
        print(f"compared: {self.compared['calls']} calls, {self.compared['folders']} folders", file=out)
        for field, ids in self.diffs.items():
            print(f"{field}: {len(ids)} differences", file=out)
            for i in ids[:10]:
                print(f"    {i}", file=out)
        print(f"{self.total()} differences", file=out)


def ident(key):
    return " | ".join("-" if part is None else str(part) for part in key)


def compare_calls(py_rows, rust_rows, report):
    """Field-by-field comparison of the shell calls; returns, per call, the classes that explained a difference."""
    py, rs = keyed(py_rows, call_key), keyed(rust_rows, call_key)
    claimed = {}
    for k in sorted(py.keys() | rs.keys(), key=str):
        if k not in rs or k not in py:
            report.diffs["call missing in " + ("rust" if k not in rs else "python")].append(ident(k))
            continue
        p, r = py[k], rs[k]
        report.compared["calls"] += 1
        rust_flags = set(r["flags"])
        tail = crlf_tail(p["text"])
        scripts = written_scripts(p["text"])
        loop = FOR_BRACE.search(p["text"]) if "looped" in p["flags"] and p["shell"] == "bash" else None
        call = {"tail": tail, "scripts": scripts, "cwd": p["cwd"], "text": p["text"], "loop": bool(loop),
                "loop_items": loop[1] if loop else "", "cand_dir": direction(p["candidates"], r["candidates"]),
                "here": here_strings(p["text"]) if p["shell"] == "powershell" else []}
        tail_info = tail_extraction(tail, p) if tail is not None else None
        active = {c for flag, c in (("truncated", "D3"), ("exhausted", "D4")) if flag in rust_flags}
        counted = set(active)   # D3 and D4 are known from the Rust flags alone
        if "script" in set(p["flags"]) | rust_flags:
            active.add("D5/D6")
        if r["extract_unresolved"] and not counted:
            counted.add("D5/D6")   # D6: counted on the Rust side only, never compared
        for cls, on in (("D2", tail is not None), ("D8", bool(BUN_INIT.search(p["text"]))), ("D9", bool(scripts)),
                        ("D10", bool(D10_CD.search(p["text"]))),
                        ("D11", p["shell"] == "bash" and bool(BRACE_EXPR.search(p["text"]))),
                        ("D12", any(impossible(c[2]) for c in p["candidates"])), ("D13", bool(call["here"]))):
            if on:
                active.add(cls)
        explained = set()
        fields = (("candidates", p["candidates"], r["candidates"]),
                  ("flags", p["flags"], sorted(rust_flags & set(COMPARED_FLAGS.values()))),
                  ("unresolved", p["unresolved"], r["unresolved"]))
        for field, pv, rv in fields:
            if pv == rv:
                continue
            diff = Diff(call, field, pv, rv, tail_info)
            by = {c for c in active if explains(c, diff)}
            if by:
                explained |= by
            else:
                report.diffs[field].append(ident(k))
        for c in counted | explained:
            report.classes["calls"][c].append(ident(k))
        claimed[k] = explained
        if call["loop"] and "D11" in explained:
            report.loops.add(k)
    return claimed


def compare_folders(py_rows, rust_rows, claimed, report):
    py, rs = keyed(py_rows, folder_key), keyed(rust_rows, folder_key)
    invented = {(row["store"], row["session"], *ref) for row in py_rows if INVENTED_DIR.search(row["folder"]) for ref in row["refs"]}
    expansions = collections.defaultdict(set)   # D11: what the prototype's brace folders expand to, by session
    for row in py_rows:
        if BRACE_EXPR.search(row["folder"]):
            expansions[(row["store"], row["session"])].update(f.lower() for f in brace_expand(row["folder"]))

    def caused(cls, sign, k, p, r, refs):
        """Whether a class can have made this folder differ this way, from the folder and its calls."""
        if cls == "D10":   # Python's option-word folder is missing in Rust; Rust's real folder comes from the same call
            return sign == FEWER and bool(p) and bool(INVENTED_DIR.search(p["folder"])) or sign == MORE and bool(refs & invented)
        if cls == "D11":   # Python's brace folder is missing; Rust's are its expansion or come from a brace-item loop
            if sign == FEWER:
                return bool(p) and bool(BRACE_EXPR.search(p["folder"]))
            return bool(r) and (r["folder"].lower() in expansions[k[:2]] or bool(refs & report.loops))
        return True

    for k in sorted(py.keys() | rs.keys(), key=str):
        p, r = py.get(k), rs.get(k)
        patched = bool(r) and any(s.startswith("patch:") for s in r["sources"])
        if patched:   # D7: the prototype ignores patches, so only what is not a patch is compared
            report.classes["folders"]["D7"].append(ident(k))
            if all(s.startswith("patch:") for s in r["sources"]):
                continue
            r = {**r, "strength": r["non_patch_strength"], "failed": r["non_patch_failed"]}
        report.compared["folders"] += 1
        refs = {(k[0], k[1], *ref) for row in (p, r) if row for ref in row["refs"]}
        explaining = set().union(*(claimed.get(ref, set()) for ref in refs))
        if p is None or r is None:
            differing = [("folder missing in " + ("rust" if r is None else "python"), FEWER if r is None else MORE)]
        else:
            differing = [(f, (r[f] or 0) - (p[f] or 0)) for f in ("strength", "failed") if p[f] != r[f]]
        for field, delta in differing:
            sign = (delta > 0) - (delta < 0)
            by = {c for c in explaining if sign in FOLDER_WAYS.get(c, EXPLAINS[c][0]) and caused(c, sign, k, p, r, refs)}
            if not by:
                report.diffs[field].append(ident(k))
            for c in by:
                report.classes["folders"][c].append(ident(k))


def parity(calls_path, rust_dir):
    import evidence
    report = Parity()
    rust_dir = Path(rust_dir)
    claimed = compare_calls(python_calls(calls_path), read_rows(rust_dir / "calls.jsonl"), report)
    compare_folders(evidence.ledger_rows(calls_path), read_rows(rust_dir / "ledger.jsonl"), claimed, report)
    return report


# Self-test: four synthetic calls (a Bash with a loop and an unresolved path, a PowerShell whose row
# has an object `canonical`, a structured Write, a Codex apply_patch with no result), the Rust output they
# must give, written by hand, and copies altered on purpose.
SYNTHETIC_CALLS = [
    {"harness": "claude-code", "store": "st", "session": "s1", "entry": "e1", "call": "c1", "tool": "Bash",
     "canonical": "bash", "cwd": r"D:\proj", "time": None, "error": None, "exit": 0,
     "input": {"command": 'mkdir -p "D:\\proj\\a" && cp -r "D:\\proj\\a" "D:\\proj\\b" && cd "D:\\proj\\c" && mkdir $UNKNOWN\n'
                          'for f in *.txt; do mkdir "D:\\proj\\loop"; done'}},
    {"harness": "claude-code", "store": "st", "session": "s1", "entry": "e2", "call": "c2", "tool": "PowerShell",
     "canonical": {"name": "power_shell"}, "cwd": "\\\\?\\D:\\proj2", "time": None, "error": False, "exit": 0,
     "result_head": "done", "parent": None, "input": {"command": r"New-Item -ItemType Directory -Path D:\proj2\made"}},
    {"harness": "claude-code", "store": "st", "session": "s1", "entry": "e3", "call": "c3", "tool": "Write",
     "canonical": "write", "cwd": r"D:\proj3", "time": None, "error": False, "exit": None,
     "result_head": "ok", "input": {"file_path": r"D:\proj3\sub\x.txt", "content": "hi"}},
    {"harness": "codex", "store": "st", "session": "s1", "entry": "e4", "call": "c4", "tool": "apply_patch",
     "canonical": None, "cwd": r"D:\proj4", "time": None, "error": False, "exit": 0,
     "input": {"_": "*** Begin Patch\n*** Add File: D:\\proj4\\sub\\f.txt\n+hi\n*** End Patch"}},
]


def synthetic_rust():
    def call(entry, cands, flags, unresolved):
        return {"store": "st", "session": "s1", "entry": entry, "call": entry.replace("e", "c"), "candidates": cands,
                "flags": flags, "unresolved": unresolved, "extract_unresolved": 0}

    def folder(path, strength, failed, source, entry):
        return {"store": "st", "session": "s1", "folder": path, "strength": strength, "failed": failed,
                "sources": [source], "refs": [[entry, entry.replace("e", "c")]],
                "non_patch_strength": None if source.startswith("patch:") else strength,
                "non_patch_failed": 0 if source.startswith("patch:") else failed}

    calls = [call("e1", [["create", None, r"D:\proj\a"], ["copy", r"D:\proj\a", r"D:\proj\b"],
                         ["create", None, r"D:\proj\loop"]], ["looped"], 1),
             call("e2", [["create", None, r"D:\proj2\made"]], [], 0)]
    ledger = [folder(r"D:\proj", 1, 0, "cwd", "e1"),
              folder(r"D:\proj\a", 3, 0, "shell:create", "e1"),
              folder(r"D:\proj\b", 3, 0, "shell:copy", "e1"),
              folder(r"D:\proj\loop", 3, 0, "shell:create", "e1"),
              folder(r"D:\proj\c", 1, 0, "cd", "e1"),
              folder(r"D:\proj2", 1, 0, "cwd", "e2"),
              folder(r"D:\proj2\made", 3, 0, "shell:create", "e2"),
              folder(r"D:\proj3", 1, 0, "cwd", "e3"),
              folder(r"D:\proj3\sub", 2, 0, "structured:Write", "e3"),
              folder(r"D:\proj4", 1, 0, "cwd", "e4"),
              folder(r"D:\proj4\sub", 2, 0, "patch:add", "e4")]  # D7: the prototype ignores patches
    return calls, ledger


def parity_self_test():
    import copy
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        calls_path = tmp / "calls.jsonl"
        calls_path.write_text("".join(json.dumps(c) + "\n" for c in SYNTHETIC_CALLS), encoding="utf-8")

        def run(calls, ledger):
            (tmp / "rust").mkdir(exist_ok=True)
            for name, rows in (("calls", calls), ("ledger", ledger)):
                (tmp / "rust" / f"{name}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
            return parity(calls_path, tmp / "rust")

        calls, ledger = synthetic_rust()
        exact = run(calls, ledger)
        exact.print()
        failures = []
        if exact.total():
            failures.append(f"exact output: {exact.total()} differences")
        if len(exact.classes["folders"]["D7"]) != 1 or any(exact.classes["calls"].values()) or exact.compared["calls"] != 2:
            failures.append("the apply_patch folder must be the only D7 and nothing else a class")

        def altered(name, change, field):
            c, l = copy.deepcopy(calls), copy.deepcopy(ledger)
            change(c, l)
            report = run(c, l)
            if {f: len(v) for f, v in report.diffs.items()} != {field: 1}:
                failures.append(f"{name}: expected one difference in {field}, got {dict(report.diffs)}")

        altered("src changed", lambda c, l: c[0]["candidates"][1].__setitem__(1, r"D:\other"), "candidates")
        altered("flag removed", lambda c, l: c[0].__setitem__("flags", []), "flags")
        altered("unresolved +1", lambda c, l: c[1].__setitem__("unresolved", 1), "unresolved")
        altered("strength lowered", lambda c, l: l[1].__setitem__("strength", 2), "strength")
        altered("failed +1", lambda c, l: l[2].__setitem__("failed", 1), "failed")
        altered("folder missing", lambda c, l: l.pop(4), "folder missing in rust")

        def mixed(name, change, expected):
            """D7 on a folder with a patch and another source: only the other source is compared."""
            c, l = copy.deepcopy(calls), copy.deepcopy(ledger)
            l[8]["sources"] = ["structured:Write", "patch:update"]
            change(l[8])
            report = run(c, l)
            got = ({f: len(v) for f, v in report.diffs.items()}, len(report.classes["folders"]["D7"]))
            if got != expected:
                failures.append(f"{name}: expected {expected}, got {got}")

        mixed("d7-mixed", lambda f: None, ({}, 2))
        mixed("d7-mixed-strength", lambda f: f.__setitem__("non_patch_strength", 3), ({"strength": 1}, 2))
        mixed("d7-mixed-failed", lambda f: f.__setitem__("non_patch_failed", 1), ({"failed": 1}, 2))

        for flag, klass in (("truncated", "D3"), ("exhausted", "D4")):  # a cut or exhausted call is a class, not a difference
            c, l = copy.deepcopy(calls), copy.deepcopy(ledger)
            c[1]["flags"], c[1]["candidates"] = [flag], []
            report = run(c, l)
            if report.total() or len(report.classes["calls"][klass]) != 1:
                failures.append(f"{flag}: expected one {klass} call and no difference, got {report.total()}")
        def case(name, command, cands, flags=(), unresolved=0, extract_unresolved=0, diffs=None, classes=None,
                 shell="bash"):
            """One shell call against hand-written Rust output: the differences and classes it must give."""
            row = {"harness": "h", "store": "st", "session": "c", "entry": name, "call": name, "tool": "Bash",
                   "canonical": shell, "cwd": r"D:\w", "input": {"command": command}}
            path = tmp / "case.jsonl"
            path.write_text(json.dumps(row) + "\n", encoding="utf-8")
            rust = [{"store": "st", "session": "c", "entry": name, "call": name, "candidates": cands,
                     "flags": list(flags), "unresolved": unresolved, "extract_unresolved": extract_unresolved}]
            report = Parity()
            compare_calls(python_calls(path), rust, report)
            got = ({f: len(v) for f, v in report.diffs.items()}, {c: len(v) for c, v in report.classes["calls"].items() if v})
            if got != (diffs or {}, classes or {}):
                failures.append(f"case {name}: expected {(diffs or {}, classes or {})}, got {got}")

        def fcase(name, command, cands, folders, unresolved=0, diffs=None, classes=None, shell="bash"):
            """One shell call and the folders Rust gave for it: the folder differences and classes it must give."""
            import evidence
            row = {"harness": "h", "store": "st", "session": "c", "entry": name, "call": name, "tool": "Bash",
                   "canonical": shell, "cwd": r"D:\w", "input": {"command": command}}
            path = tmp / "fcase.jsonl"
            path.write_text(json.dumps(row) + "\n", encoding="utf-8")
            rust_calls = [{"store": "st", "session": "c", "entry": name, "call": name, "candidates": cands, "flags": [],
                           "unresolved": unresolved, "extract_unresolved": 0}]
            rust_ledger = [{"store": "st", "session": "c", "folder": f, "strength": st, "failed": 0, "sources": [src],
                            "refs": [[ref or name, ref or name]], "non_patch_strength": st, "non_patch_failed": 0}
                           for f, st, src, ref in folders]
            report = Parity()
            claimed = compare_calls(python_calls(path), rust_calls, report)
            report.diffs.clear()   # only the folders are judged here
            compare_folders(evidence.ledger_rows(path), rust_ledger, claimed, report)
            got = ({f: len(v) for f, v in report.diffs.items()}, {c: len(v) for c, v in report.classes["folders"].items() if v})
            if got != (diffs or {}, classes or {}):
                failures.append(f"case {name}: expected {(diffs or {}, classes or {})}, got {got}")

        mk = lambda p: ["create", None, "D:\\x\\" + p]  # noqa: E731
        d5 = '''python -c "import os; os.makedirs('D:/x/a'); print(\\"os.makedirs('D:/x/b')\\")"'''
        d2 = "cat > a.txt <<EOF\r\nhello\r\nEOF\r\nmkdir D:\\x\\after"
        d9 = "cat > s.py <<'EOF'\nimport os\nos.makedirs('D:/x/n')\nEOF\nbash -c \"python s.py\""
        wr = ["write", None, r"D:\w\a.txt"]
        ws = ["write", None, r"D:\w\s.py"]
        # D5/D6: the script string callers are not found, the arguments are counted on the Rust side only
        case("d5-fewer", d5, [mk("a")], ["script"], classes={"D5/D6": 1})
        case("d6-count", d5, [mk("a"), mk("b")], ["script"], extract_unresolved=2, classes={"D5/D6": 1})
        case("d5-extra", d5, [mk("a"), mk("zzz")], ["script"], extract_unresolved=1, diffs={"candidates": 1}, classes={"D5/D6": 1})
        case("d5-wrong-dst", d5, [mk("a"), mk("c")], ["script"], diffs={"candidates": 1})
        case("d6-flag", d5, [mk("a"), mk("b")], [], extract_unresolved=1, diffs={"flags": 1}, classes={"D5/D6": 1})
        case("d6-unresolved", d5, [mk("a"), mk("b")], ["script"], unresolved=1, extract_unresolved=1, diffs={"unresolved": 1}, classes={"D5/D6": 1})
        # D2: Python swallows the commands after `EOF\r`; only Rust-only candidates from that tail are explained
        case("d2", d2, [wr, mk("after")], classes={"D2": 1})
        case("d2-unrelated", d2, [wr, mk("unrelated")], diffs={"candidates": 1})
        case("d2-fewer", d2, [], diffs={"candidates": 1})
        # D8: `bun init <dir>` gives a create on the Rust side
        case("d8", r"bun init D:\x\app", [mk("app")], classes={"D8": 1})
        case("d8-not-create", r"bun init D:\x\app", [["write", None, r"D:\x\app"]], diffs={"candidates": 1})
        case("d8-src", r"bun init D:\x\app", [["create", "D:\\x", r"D:\x\app"]], diffs={"candidates": 1})
        # D9: a nested shell runs a script written earlier in the same command
        case("d9", d9, [ws, mk("n")], classes={"D9": 1})
        case("d9-unrelated", d9, [ws], unresolved=1, diffs={"unresolved": 1})
        case("d9-no-script", 'bash -c "python s.py"', [mk("n")], diffs={"candidates": 1})
        case("d9-redirect", 'mkdir D:/x/q 2>nul; bash -c "echo hi"', [mk("q"), mk("n")], diffs={"candidates": 1})
        # D2 also explains the flags and unresolved paths, but only those of the swallowed tail
        d2_script = "cat > a.txt <<EOF\r\nhello\r\nEOF\r\npython -c \"import os; os.makedirs('D:/x/s')\""
        d2_unknown = "cat > a.txt <<EOF\r\nhello\r\nEOF\r\nmkdir $UNK"
        case("d2-flag", d2_script, [wr, mk("s")], ["script"], classes={"D2": 1})
        case("d2-flag-unrelated", d2, [wr, mk("after")], ["script"], diffs={"flags": 1}, classes={"D2": 1})
        case("d2-unresolved", d2_unknown, [wr], unresolved=1, classes={"D2": 1})
        case("d2-unresolved-unrelated", d2.replace("D:\\x\\after", "D:/x/after"), [wr, mk("after")], unresolved=1, diffs={"unresolved": 1}, classes={"D2": 1})
        # D9 explains only what the written script holds
        case("d9-extra-unrelated", d9, [ws, mk("n"), mk("zzz")], diffs={"candidates": 1})
        # D10: an option word of `cd` is not a directory; `cd -` leaves the directory unknown
        case("d10-dash", "cd /d/a && cd - && mkdir out", [], unresolved=2, classes={"D10": 1})
        case("d10-option", "cd -P /d/a && mkdir out", [["create", None, r"D:\a\out"]], classes={"D10": 1})
        case("d10-dashdash", "cd -- /d/a && mkdir out", [["create", None, r"D:\a\out"]], classes={"D10": 1})
        case("d10-powershell", "cd -; mkdir out", [], unresolved=2, classes={"D10": 1}, shell="power_shell")
        case("d10-fewer-real", "cd - && mkdir /d/a /d/b", [["create", None, r"D:\a"]], unresolved=1, diffs={"candidates": 1}, classes={"D10": 1})
        case("d10-inactive", "mkdir D:/x/a; mkdir D:/x/b", [mk("a")], diffs={"candidates": 1})
        # D11: a Bash brace expression is the paths it expands to; past the limit it is unknown and counted
        case("d11", "mkdir src/{api,ui}", [["create", None, r"D:\w\src\api"], ["create", None, r"D:\w\src\ui"]], classes={"D11": 1})
        case("d11-seq", "mkdir d{1..2}", [["create", None, r"D:\w\d1"], ["create", None, r"D:\w\d2"]], classes={"D11": 1})
        case("d11-copy", "cp f{,.bak} D:/x", [["copy", r"D:\w\f", r"D:\x"], ["copy", r"D:\w\f.bak", r"D:\x"]], classes={"D11": 1})
        case("d11-limit", "mkdir src/{api,ui}", [], unresolved=1, classes={"D11": 1})
        case("d11-other-paths", "mkdir src/{api,ui}", [["create", None, r"D:\w\src\api"], ["create", None, r"D:\w\src\other"]], diffs={"candidates": 1})
        case("d11-no-braces", "mkdir src/plain", [], diffs={"candidates": 1})
        case("d11-powershell", "mkdir src/{api,ui}", [["create", None, r"D:\w\src\api"], ["create", None, r"D:\w\src\ui"]], diffs={"candidates": 1}, shell="power_shell")
        # D10 explains Python's invented option-word directories, not a Rust surplus or fewer unresolved paths
        case("d10-rust-more", "cd -P /d/a && mkdir out", [["create", None, r"D:\a\out"], mk("zzz")], diffs={"candidates": 1})
        case("d10-unresolved-fewer", "mkdir $UNK; cd -P /d/a", [], diffs={"unresolved": 1})
        # D11 also covers a `for` loop over brace items (Rust expands it, so no `looped` flag) and the limit
        loop = "for d in a b{1,2}; do mkdir out/$d; done"
        loop_rust = [["create", None, r"D:\w\out\a"], ["create", None, r"D:\w\out\b1"], ["create", None, r"D:\w\out\b2"]]
        case("d11-loop", loop, loop_rust, classes={"D11": 1})
        case("d11-loop-unrelated", loop, loop_rust + [mk("zzz")], diffs={"candidates": 1}, classes={"D11": 1})
        case("d11-loop-no-braces", "for f in *.txt; do mkdir out/$f; done", [], unresolved=1, diffs={"flags": 1})
        case("d11-unresolved-more", "mkdir src/{api,ui}", [["create", None, r"D:\w\src\api"], ["create", None, r"D:\w\src\ui"]],
             unresolved=1, diffs={"unresolved": 1}, classes={"D11": 1})
        # D12: a path with characters Windows cannot hold is unresolved, never a candidate
        case("d12", 'mkdir "a\nb" /d/ok', [["create", None, r"D:\ok"]], unresolved=1, classes={"D12": 1})
        case("d12-angle", "mkdir 'a<b' /d/ok", [["create", None, r"D:\ok"]], unresolved=1, classes={"D12": 1})
        case("d12-extra", 'mkdir "a\nb" /d/ok', [["create", None, r"D:\ok"], mk("zzz")], unresolved=1, diffs={"candidates": 1}, classes={"D12": 1})
        case("d12-inactive", "mkdir D:/x/a; mkdir D:/x/b", [mk("a")], unresolved=1, diffs={"candidates": 1, "unresolved": 1})
        # D13: the text inside a PowerShell here-string is one word, not paths
        d13 = "Set-Content D:/a/z.ts @'\nit's a\nmkdir D:/no\n'@"
        wz = ["write", None, r"D:\a\z.ts"]
        case("d13", d13, [wz], classes={"D13": 1}, shell="powershell")
        case("d13-python-extra", "mkdir D:/q; " + d13, [wz], diffs={"candidates": 1}, shell="powershell")
        case("d13-rust-extra", d13, [wz, mk("zzz")], diffs={"candidates": 1}, shell="powershell")
        case("d13-no-here-string", "Set-Content D:/a/z.ts; mkdir D:/no", [wz], diffs={"candidates": 1}, shell="powershell")
        case("d13-bash", d13, [wz], diffs={"candidates": 1})
        # Folders: each class explains only the folders it can make differ
        wf = lambda f, s=3, src="shell:create", ref=None: (f, s, src, ref)  # noqa: E731
        cwd = wf("D:\\w", 1, "cwd")
        fcase("f-d11", "mkdir D:/w/src/{api,ui}/x", [["create", None, r"D:\w\src\api\x"], ["create", None, r"D:\w\src\ui\x"]],
              [cwd, wf(r"D:\w\src\api\x"), wf(r"D:\w\src\ui\x")], classes={"D11": 3})
        fcase("f-d11-other", "mkdir D:/w/src/{api,ui}/x", [["create", None, r"D:\w\src\api\x"], ["create", None, r"D:\w\src\ui\x"]],
              [cwd, wf(r"D:\w\src\api\x"), wf(r"D:\w\src\ui\x"), wf(r"D:\w\other")], diffs={"folder missing in python": 1}, classes={"D11": 3})
        fcase("f-d11-loop", "for d in a b{1,2}; do mkdir out/$d/x; done",
              [["create", None, r"D:\w\out\a\x"], ["create", None, r"D:\w\out\b1\x"], ["create", None, r"D:\w\out\b2\x"]],
              [cwd, wf(r"D:\w\out\a\x"), wf(r"D:\w\out\b1\x"), wf(r"D:\w\out\b2\x")], classes={"D11": 3})
        fcase("f-d10", "cd -P /d/a && mkdir out", [["create", None, r"D:\a\out"]],
              [cwd, wf(r"D:\a\out"), wf("D:\\a", 1, "cd")], classes={"D10": 4})
        fcase("f-d10-unrelated", "cd -P /d/a && mkdir out", [["create", None, r"D:\a\out"]],
              [cwd, wf(r"D:\a\out"), wf("D:\\a", 1, "cd"), wf(r"D:\zz", ref="other")], diffs={"folder missing in python": 1}, classes={"D10": 4})
        fcase("f-d10-inactive", "mkdir D:/w/q", [["create", None, r"D:\w\q"]], [cwd, wf(r"D:\w\q"), wf(r"D:\w\zz")],
              diffs={"folder missing in python": 1})
        fcase("f-d12", 'mkdir "D:/w/a\nb/x"', [], [cwd], unresolved=1, classes={"D12": 1})
        fcase("f-d13", d13, [wz], [cwd, wf("D:\\a", 2, "shell:write")], shell="powershell", classes={"D13": 1})
        # D3/D4: a cut or exhausted call can only give fewer candidates
        case("d3", "mkdir D:/x/a; mkdir D:/x/b", [mk("a")], ["truncated"], classes={"D3": 1})
        case("d3-more", "mkdir D:/x/a", [mk("a"), mk("b")], ["truncated"], diffs={"candidates": 1}, classes={"D3": 1})
        case("d4", "mkdir D:/x/a; mkdir D:/x/b", [mk("a")], ["exhausted"], classes={"D4": 1})
        if failures:
            print("SELF-TEST FAILED:\n  " + "\n  ".join(failures))
            sys.exit(1)
        print("self-test passed: exact output 0 differences, 9 altered copies, 60 class cases (D2-D13, positive and negative)")


def cmd_parity(args):
    home = args.home or HOME
    env = {"home": home, "temp": os.path.join(home, "AppData", "Local", "Temp")}
    ENV.update(env)
    import evidence
    evidence.ENV.update(env)  # evidence.py imports its own copy of this module when this one runs as a script
    if args.self_test:
        return parity_self_test()
    report = parity(args.calls, args.rust)
    report.print()
    sys.exit(1 if report.total() else 0)


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("sample"); a.add_argument("--calls", required=True); a.add_argument("--out", required=True)
    a.add_argument("--per-stratum", type=int, default=60); a.add_argument("--seed", type=int, default=7)
    a.add_argument("--exclude", nargs="*")
    b = sub.add_parser("model"); b.add_argument("--sample", required=True); b.add_argument("--server", required=True)
    b.add_argument("--model", required=True); b.add_argument("--threads", type=int, default=4)
    b.add_argument("--port", type=int, default=8765)
    c = sub.add_parser("score"); c.add_argument("--sample", required=True); c.add_argument("--labels", required=True)
    d = sub.add_parser("parity", help="compare the Rust probe's output with the prototype")
    d.add_argument("--calls"); d.add_argument("--rust"); d.add_argument("--home")
    d.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.cmd == "parity" and not args.self_test and not (args.calls and args.rust):
        ap.error("parity needs --calls and --rust, or --self-test")
    {"sample": cmd_sample, "model": cmd_model, "score": cmd_score, "parity": cmd_parity}[args.cmd](args)


if __name__ == "__main__":
    main()
