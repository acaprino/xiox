"""The benchmark on real sessions: shell calls collected by harvest.py.

    python real.py sample --calls calls.jsonl --out sample.json [--per-stratum 60] [--seed 7]
        deterministic extraction on every shell call; a stratified sample for the model and labels
    python real.py model --sample sample.json --server <llama-server.exe> --model <gguf> [--threads 4]
        the model on the sample (same prompt, schema and guard as bench.py)
    python real.py score --sample sample.json --labels labels.json
        labels.json: {event id: [[op, resolved dst], ...]}, written by hand from the sample
    python real.py parity --calls calls.jsonl --rust <dir> [--home <dir>]
        the Rust probe's output (crates/xiox-project/examples/shell_probe.rs) against this prototype,
        field by field; declared divergences D2-D9 are counted apart. --self-test needs neither.

Strata: det-found (the deterministic extractor gives a candidate), det-unsure (it flags a loop or
script, or a path it cannot resolve), det-empty (neither). Real commands stay in the scratch files."""
import argparse
import collections
import json
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
WRITES_FILE = re.compile(r"<<|[^>]>[^>&]")
CLASS_ORDER = ("D2", "D3", "D4", "D5/D6", "D7", "D8", "D9")


def crlf_heredoc(text):
    """D2: a heredoc delimiter followed by a carriage return, which the prototype does not close on."""
    return any(re.search(rf"(?m)^[ \t]*{re.escape(m[2])}\r$", text) for m in HEREDOC.finditer(text))


def text_classes(text):
    """Declared divergences recognised in the command text: D2, D8, D9 (a nested shell beside a file write)."""
    found = {"D2"} if crlf_heredoc(text) else set()
    if BUN_INIT.search(text):
        found.add("D8")
    if NESTED_SHELL.search(text) and WRITES_FILE.search(text):
        found.add("D9")
    return found


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
        rows.append({"store": c["store"], "session": c["session"], "entry": c.get("entry"), "call": c.get("call"),
                     "candidates": [[o["op"], o["src"], o["dst"]] for o in cands],
                     "flags": sorted(COMPARED_FLAGS[f] for f in flags if f in COMPARED_FLAGS),
                     "unresolved": unresolved, "text": s[1]})
    return rows


class Parity:
    """Differences by field (ids only) and the declared-divergence classes, for calls and for folders."""

    def __init__(self):
        self.diffs = collections.defaultdict(list)
        self.classes = {kind: collections.defaultdict(list) for kind in ("calls", "folders")}
        self.compared = collections.Counter()

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
    """Field-by-field comparison of the shell calls; returns the classes of every call by key."""
    py, rs = keyed(py_rows, call_key), keyed(rust_rows, call_key)
    claimed = {}
    for k in sorted(py.keys() | rs.keys(), key=str):
        if k not in rs or k not in py:
            report.diffs["call missing in " + ("rust" if k not in rs else "python")].append(ident(k))
            continue
        p, r = py[k], rs[k]
        report.compared["calls"] += 1
        rust_flags = set(r["flags"])
        classes = {c for flag, c in (("truncated", "D3"), ("exhausted", "D4")) if flag in rust_flags}
        if r["extract_unresolved"] and not classes:
            classes.add("D5/D6")  # script arguments the prototype does not count, compared on the Rust side only
        if not classes:
            fields = {"candidates": p["candidates"] != r["candidates"],
                      "flags": p["flags"] != sorted(rust_flags & set(COMPARED_FLAGS.values())),
                      "unresolved": p["unresolved"] != r["unresolved"]}
            if any(fields.values()):
                classes = text_classes(p["text"])
                if not classes and "script" in set(p["flags"]) | rust_flags and [f for f, d in fields.items() if d] == ["candidates"]:
                    classes.add("D5/D6")  # callers searched inside script strings
                if not classes:
                    for field, differs in fields.items():
                        if differs:
                            report.diffs[field].append(ident(k))
        for c in classes:
            report.classes["calls"][c].append(ident(k))
        claimed[k] = classes
    return claimed


def compare_folders(py_rows, rust_rows, claimed, report):
    py, rs = keyed(py_rows, folder_key), keyed(rust_rows, folder_key)
    for k in sorted(py.keys() | rs.keys(), key=str):
        p, r = py.get(k), rs.get(k)
        if r and any(s.startswith("patch:") for s in r["sources"]):
            report.classes["folders"]["D7"].append(ident(k))
            continue
        report.compared["folders"] += 1
        refs = {(k[0], k[1], *ref) for row in (p, r) if row for ref in row["refs"]}
        explaining = set().union(*(claimed.get(ref, set()) for ref in refs))
        if p is None or r is None:
            fields = ["folder missing in " + ("rust" if r is None else "python")]
        else:
            fields = [f for f in ("strength", "failed") if p[f] != r[f]]
        if fields and explaining:
            for c in explaining:
                report.classes["folders"][c].append(ident(k))
        else:
            for f in fields:
                report.diffs[f].append(ident(k))


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
                "sources": [source], "refs": [[entry, entry.replace("e", "c")]]}

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

        for flag, klass in (("truncated", "D3"), ("exhausted", "D4")):  # a cut or exhausted call is a class, not a difference
            c, l = copy.deepcopy(calls), copy.deepcopy(ledger)
            c[1]["flags"], c[1]["candidates"] = [flag], []
            report = run(c, l)
            if report.total() or len(report.classes["calls"][klass]) != 1:
                failures.append(f"{flag}: expected one {klass} call and no difference, got {report.total()}")
        if failures:
            print("SELF-TEST FAILED:\n  " + "\n  ".join(failures))
            sys.exit(1)
        print("self-test passed: exact output 0 differences, 6 altered copies, D3/D4 and D7 classes")


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
