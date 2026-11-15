"""The benchmark on real sessions: shell calls collected by harvest.py.

    python real.py sample --calls calls.jsonl --out sample.json [--per-stratum 60] [--seed 7]
        deterministic extraction on every shell call; a stratified sample for the model and labels
    python real.py model --sample sample.json --server <llama-server.exe> --model <gguf> [--threads 4]
        the model on the sample (same prompt, schema and guard as bench.py)
    python real.py score --sample sample.json --labels labels.json
        labels.json: {event id: [[op, resolved dst], ...]}, written by hand from the sample

Strata: det-found (the deterministic extractor gives a candidate), det-unsure (it flags a loop or
script, or a path it cannot resolve), det-empty (neither). Real commands stay in the scratch files."""
import argparse
import collections
import json
import os
import random
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
    args = ap.parse_args()
    {"sample": cmd_sample, "model": cmd_model, "score": cmd_score}[args.cmd](args)


if __name__ == "__main__":
    main()
